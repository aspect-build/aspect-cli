package lint

import (
	"context"
	"fmt"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"strings"
	"time"

	"aspect.build/cli/bazel/buildeventstream"
	"golang.org/x/sync/errgroup"
)

// ResultForLabel aggregates the relevant files we find in the BEP for
type ResultForLabel struct {
	exitCodeFile *buildeventstream.File
	reportFile   *buildeventstream.File
	patchFile    *buildeventstream.File
	linter       string
}

type LintBEPHandler struct {
	ctx                   context.Context
	reports               map[string]*buildeventstream.NamedSetOfFiles
	workspaceRoot         string
	handleResultsErrgroup *errgroup.Group
	resultsByLabel        map[string]*ResultForLabel
}

func newLintBEPHandler(ctx context.Context, workspaceRoot string, handleResultsErrgroup *errgroup.Group) *LintBEPHandler {
	return &LintBEPHandler{
		ctx:                   ctx,
		reports:               make(map[string]*buildeventstream.NamedSetOfFiles),
		resultsByLabel:        make(map[string]*ResultForLabel),
		workspaceRoot:         workspaceRoot,
		handleResultsErrgroup: handleResultsErrgroup,
	}
}

func (runner *LintBEPHandler) readBEPFile(file *buildeventstream.File) (string, error) {
	resultsFile := ""

	switch f := file.File.(type) {
	case *buildeventstream.File_Uri:
		uri, err := url.Parse(f.Uri)
		if err != nil {
			return "", fmt.Errorf("unable to parse URI %s: %v", f.Uri, err)
		}
		if uri.Scheme == "file" {
			resultsFile = filepath.Clean(uri.Path)
		} else if uri.Scheme == "bytestream" {
			if strings.HasSuffix(uri.Path, "/0") {
				// No reason to read an empty results file from disk
				return "", nil
			}
			// Because we set --experimental_remote_download_regex, we can depend on the results file being
			// in the output tree even when using a remote cache with build without the bytes.
			resultsFile = path.Join(runner.workspaceRoot, path.Join(file.PathPrefix...), file.Name)
		} else {
			return "", fmt.Errorf("unsupported BES file uri %v", f.Uri)
		}
	default:
		return "", fmt.Errorf("unsupported BES file type")
	}

	start := time.Now()
	for {
		// TODO: also check that the bazel remote cache downloader is not still writing
		// to the results file
		_, err := os.Stat(resultsFile)
		if err != nil {
			// if more than 60s has passed then give up
			// TODO: make this timeout configurable
			if time.Since(start) > 60*time.Second {
				return "", fmt.Errorf("failed to read lint results file %s: %v", resultsFile, err)
			}
		} else {
			buf, err := os.ReadFile(resultsFile)
			if err != nil {
				return "", fmt.Errorf("failed to read lint results file %s: %v", resultsFile, err)
			}
			return string(buf), nil
		}
		// we're in a go routine so yield for 100ms and try again
		// TODO: watch the file system for the file creation instead of polling
		t := time.NewTimer(time.Millisecond * 100)
		select {
		case <-runner.ctx.Done():
			t.Stop()
			return "", fmt.Errorf("failed to read lint results file %s: interrupted", resultsFile)
		case <-t.C:
		}
	}
}

func (runner *LintBEPHandler) BEPEventCallback(event *buildeventstream.BuildEvent) error {
	switch event.Payload.(type) {

	case *buildeventstream.BuildEvent_NamedSetOfFiles:
		// Assert no collisions
		namedSetId := event.Id.GetNamedSet().GetId()
		if runner.reports[namedSetId] != nil {
			return fmt.Errorf("duplicate file set id: %s", namedSetId)
		}

		// Record report file sets
		// TODO: are we collecting too much? Don't we need to filter on the "report" output group?
		runner.reports[namedSetId] = event.GetNamedSetOfFiles()

	case *buildeventstream.BuildEvent_Completed:
		label := event.Id.GetTargetCompleted().GetLabel()

		for _, outputGroup := range event.GetCompleted().OutputGroup {
			for _, fileSetId := range outputGroup.FileSets {
				if fileSet := runner.reports[fileSetId.Id]; fileSet != nil {
					runner.reports[fileSetId.Id] = nil
					result := runner.resultsByLabel[label]
					if result == nil {
						result = &ResultForLabel{}
						runner.resultsByLabel[label] = result
					}

					for _, file := range fileSet.GetFiles() {
						if outputGroup.Name == LINT_PATCH_GROUP {
							result.patchFile = file

							// Parse the filename convention that rules_lint has for report files.
							// path/to/linter.target_name.aspect_rules_lint.report -> linter
							result.linter = strings.SplitN(filepath.Base(file.Name), ".", 2)[0]
						} else if outputGroup.Name == LINT_REPORT_GROUP && strings.HasSuffix(file.Name, ".report") {
							result.reportFile = file
						} else if outputGroup.Name == LINT_REPORT_GROUP && strings.HasSuffix(file.Name, ".exit_code") {
							result.exitCodeFile = file
						}
					}
				}
			}
		}
	}

	return nil
}
