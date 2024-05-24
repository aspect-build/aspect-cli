/*
 * Copyright 2023 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package lint

import (
	"context"
	"fmt"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"aspect.build/cli/bazel/buildeventstream"
	"golang.org/x/sync/errgroup"
)

// ResultForLabel aggregates the relevant files we find in the BEP for
type ResultForLabel struct {
	label        string
	exitCodeFile *buildeventstream.File
	reportFile   *buildeventstream.File
	patchFile    *buildeventstream.File
	linter       string
}

type LintBEPHandler struct {
	ctx                   context.Context
	namedSets             map[string]*buildeventstream.NamedSetOfFiles
	workspaceRoot         string
	handleResultsErrgroup *errgroup.Group
	resultsByLabel        map[string]*ResultForLabel
	lintHandlers          []LintHandler
}

func newLintBEPHandler(ctx context.Context, workspaceRoot string, handleResultsErrgroup *errgroup.Group, lintHandlers []LintHandler) *LintBEPHandler {
	return &LintBEPHandler{
		ctx:                   ctx,
		namedSets:             make(map[string]*buildeventstream.NamedSetOfFiles),
		resultsByLabel:        make(map[string]*ResultForLabel),
		workspaceRoot:         workspaceRoot,
		handleResultsErrgroup: handleResultsErrgroup,
		lintHandlers:          lintHandlers,
	}
}

func (runner *LintBEPHandler) readBEPFile(file *buildeventstream.File) ([]byte, error) {
	resultsFile := ""

	switch f := file.File.(type) {
	case *buildeventstream.File_Uri:
		uri, err := url.Parse(f.Uri)
		if err != nil {
			return nil, fmt.Errorf("unable to parse URI %s: %v", f.Uri, err)
		}
		if uri.Scheme == "file" {
			resultsFile = filepath.Clean(uri.Path)
		} else if uri.Scheme == "bytestream" {
			if strings.HasSuffix(uri.Path, "/0") {
				// No reason to read an empty results file from disk
				return nil, nil
			}
			// Because we set --experimental_remote_download_regex, we can depend on the results file being
			// in the output tree even when using a remote cache with build without the bytes.
			resultsFile = path.Join(runner.workspaceRoot, path.Join(file.PathPrefix...), file.Name)
		} else {
			return nil, fmt.Errorf("unsupported BES file uri %v", f.Uri)
		}
	default:
		return nil, fmt.Errorf("unsupported BES file type")
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
				return nil, fmt.Errorf("failed to read lint results file %s: %v", resultsFile, err)
			}
		} else {
			buf, err := os.ReadFile(resultsFile)
			if err != nil {
				return nil, fmt.Errorf("failed to read lint results file %s: %v", resultsFile, err)
			}
			return buf, nil
		}
		// we're in a go routine so yield for 100ms and try again
		// TODO: watch the file system for the file creation instead of polling
		t := time.NewTimer(time.Millisecond * 100)
		select {
		case <-runner.ctx.Done():
			t.Stop()
			return nil, fmt.Errorf("failed to read lint results file %s: interrupted", resultsFile)
		case <-t.C:
		}
	}
}

func (runner *LintBEPHandler) bepEventCallback(event *buildeventstream.BuildEvent) error {
	switch event.Payload.(type) {

	case *buildeventstream.BuildEvent_NamedSetOfFiles:
		runner.namedSets[event.Id.GetNamedSet().Id] = event.GetNamedSetOfFiles()

	case *buildeventstream.BuildEvent_Completed:
		label := event.Id.GetTargetCompleted().GetLabel()

		for _, outputGroup := range event.GetCompleted().OutputGroup {
			for _, fileSetId := range outputGroup.FileSets {
				if fileSet := runner.namedSets[fileSetId.Id]; fileSet != nil {
					runner.namedSets[fileSetId.Id] = nil
					result := runner.resultsByLabel[label]
					if result == nil {
						result = &ResultForLabel{label: label}
						runner.resultsByLabel[label] = result
					}

					for _, file := range fileSet.GetFiles() {
						if outputGroup.Name == LINT_PATCH_GROUP {
							result.patchFile = file
						} else if outputGroup.Name == LINT_REPORT_GROUP {
							if strings.HasSuffix(file.Name, ".report") {
								result.reportFile = file

								// Parse the filename convention that rules_lint has for report files.
								// path/to/linter.target_name.aspect_rules_lint.report -> linter
								s := strings.Split(filepath.Base(file.Name), ".")
								if len(s) > 2 {
									result.linter = s[len(s)-2]
								}
							} else if strings.HasSuffix(file.Name, ".exit_code") {
								result.exitCodeFile = file
							}
						}
					}

					if outputGroup.Name == LINT_PATCH_GROUP {
						runner.lintHandlersPatch(result)
					} else if outputGroup.Name == LINT_REPORT_GROUP {
						runner.lintHandlersReport(result)
					}
				}
			}
		}
	}

	return nil
}

func (runner *LintBEPHandler) lintHandlersPatch(result *ResultForLabel) {
	if len(runner.lintHandlers) == 0 {
		return
	}

	if result.patchFile == nil {
		return
	}

	// async handling of this lint patch
	func(label string, linter string, patchFile *buildeventstream.File) {
		runner.handleResultsErrgroup.Go(func() error {
			patch, err := runner.readBEPFile(patchFile)
			if err != nil {
				return fmt.Errorf("failed to read patch file for target %s from linter %s: %v", label, linter, err)
			}
			for _, h := range runner.lintHandlers {
				if err = h.Patch(label, linter, patch); err != nil {
					return fmt.Errorf("failed to handle patch for target %s from linter %s: %v", label, linter, err)
				}
			}
			return nil
		})
	}(result.label, result.linter, result.patchFile)
}

func (runner *LintBEPHandler) lintHandlersReport(result *ResultForLabel) {
	if len(runner.lintHandlers) == 0 {
		return
	}

	if result.reportFile == nil {
		return
	}

	// async handling of this lint result
	func(label string, linter string, reportFile *buildeventstream.File, exitCodeFile *buildeventstream.File) {
		runner.handleResultsErrgroup.Go(func() error {
			report, err := runner.readBEPFile(reportFile)
			if err != nil {
				return fmt.Errorf("failed to read report file for target %s from linter %s: %v", label, linter, err)
			}
			exitCode := 0
			if exitCodeFile != nil {
				exitCodeBytes, err := runner.readBEPFile(exitCodeFile)
				if err != nil {
					return fmt.Errorf("failed to read exit code file for target %s from linter %s: %v", label, linter, err)
				}
				targetExitCode, err := strconv.Atoi(strings.TrimSpace(string(exitCodeBytes)))
				if err != nil {
					return fmt.Errorf("failed to parse exit code as integer for target %s from linter %s: %v", label, linter, err)
				}
				if targetExitCode > 0 {
					exitCode = 1
				}
			}
			for _, h := range runner.lintHandlers {
				if err = h.Report(label, linter, report, exitCode); err != nil {
					return fmt.Errorf("failed to handle report for target %s from linter %s: %v", label, linter, err)
				}
			}
			return nil
		})
	}(result.label, result.linter, result.reportFile, result.exitCodeFile)
}
