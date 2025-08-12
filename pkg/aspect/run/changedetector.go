package run

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path"
	"slices"
	"strings"
	"time"

	buildeventstream "github.com/aspect-build/aspect-cli/bazel/buildeventstream"
	"github.com/aspect-build/aspect-cli/pkg/ibp"
	"google.golang.org/protobuf/encoding/protodelim"
)

type ExecLogEntry struct {
	/*
		"inputs": [{
			"path": "bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg/index.js",
			"digest": {
			"hash": "ae73d02cc249608e1ade5f29faf00c673fa4079ae527ab2e03ad4c0265b13553",
			"sizeBytes": "153",
			"hashFunctionName": "SHA-256"
			},
			"isTool": false,
			"symlinkTargetPath": ""
		}, {
			"path": "bazel-out/darwin_arm64-fastbuild/bin/mypkg/pkg/package.json",
			"digest": {
			"hash": "0de716f58c78b84a6cad44209f15734e0046adf27c2025da67f4d9ecc660cb7b",
			"sizeBytes": "83",
			"hashFunctionName": "SHA-256"
			},
			"isTool": false,
			"symlinkTargetPath": ""
		}, {
			"path": "external/aspect_bazel_lib~~toolchains~copy_directory_darwin_arm64/copy_directory",
			"digest": {
			"hash": "7987b4ed6e0e519256b3b0ea6ef35b5aad6dd10b50fcc70467ae20b902b00d2c",
			"sizeBytes": "1544418",
			"hashFunctionName": "SHA-256"
			},
			"isTool": true,
			"symlinkTargetPath": ""
		}],

		"listedOutputs": ["bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@mycorp+mypkg@0.0.0/node_modules/@mycorp/mypkg"],

		"actualOutputs": [{
			"path": "bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@mycorp+mypkg@0.0.0/node_modules/@mycorp/mypkg/index.js",
			"digest": {
			"hash": "ae73d02cc249608e1ade5f29faf00c673fa4079ae527ab2e03ad4c0265b13553",
			"sizeBytes": "153",
			"hashFunctionName": "SHA-256"
			},
			"isTool": false,
			"symlinkTargetPath": ""
		}, {
			"path": "bazel-out/darwin_arm64-fastbuild/bin/node_modules/.aspect_rules_js/@mycorp+mypkg@0.0.0/node_modules/@mycorp/mypkg/package.json",
			"digest": {
			"hash": "0de716f58c78b84a6cad44209f15734e0046adf27c2025da67f4d9ecc660cb7b",
			"sizeBytes": "83",
			"hashFunctionName": "SHA-256"
			},
			"isTool": false,
			"symlinkTargetPath": ""
		}],
	*/

	ListedOutputs []string `json:"listedOutputs"`
	Outputs       []struct {
		Path string `json:"path"`
	} `json:"actualOutputs"`
}

type ChangeDetector struct {
	workspaceDir string
	execlogFile  *os.File
	besFile      *os.File

	// Current known state of sources
	sourcesInfo ibp.SourceInfoMap

	// Changes detected to the sources since the last cycleChanges() call.
	cycleSourceChanges ibp.SourceInfoMap

	targetTags           []string
	targetLabel          string
	targetExecutablePath string
	localExecroot        string
}

func newChangeDetector(workspaceDir string) (*ChangeDetector, error) {
	execlog, err := os.CreateTemp(os.TempDir(), fmt.Sprintf("aspect-watch-%v-execlog-*.json", os.Getpid()))
	if err != nil {
		return nil, err
	}
	bes, err := os.CreateTemp(os.TempDir(), fmt.Sprintf("aspect-watch-%v-bes-*.proto", os.Getpid()))
	if err != nil {
		return nil, err
	}
	return &ChangeDetector{
		workspaceDir:         workspaceDir,
		execlogFile:          execlog,
		besFile:              bes,
		targetTags:           []string{},
		targetLabel:          "",
		targetExecutablePath: "",
	}, nil
}

func (cd *ChangeDetector) Close() error {
	return errors.Join(
		cd.execlogFile.Close(),
		cd.besFile.Close(),
		os.Remove(cd.execlogFile.Name()),
		os.Remove(cd.besFile.Name()),
	)
}

func (cd *ChangeDetector) bazelFlags(trackChanges bool) []string {
	flags := []string{}

	if trackChanges {
		// TODO: maybe use a more compact format for better performance?
		flags = append(flags, "--execution_log_json_file", cd.execlogFile.Name(), "--noexecution_log_sort")
	}

	if !cd.hasTargetBuildEventInfo() {
		flags = append(flags, "--build_event_binary_file", cd.besFile.Name(), "--build_event_binary_file_upload_mode=fully_async")
	}

	return flags
}

func (cd *ChangeDetector) hasTargetBuildEventInfo() bool {
	return !(cd.targetLabel == "" || cd.targetExecutablePath == "" || cd.localExecroot == "")
}

func (cd *ChangeDetector) processBES(ctx context.Context) error {
	r, err := os.Open(cd.besFile.Name())
	if err != nil {
		return err
	}
	defer r.Close()

	reader := bufio.NewReader(r)

	// It is expected that the BES output will contain new information every
	// 20 seconds to avoid deadlocking the program.
	// If for some reason Bazel can't produce BES data every 20 seconds, it
	// might be dead already or so slow that it can be considered dead.
	// This can be adjusted as needed in future if needed.
	timeoutd := 20 * time.Second
	timeout := time.After(timeoutd)

	namedSets := make(map[string][]*buildeventstream.File, 0)

	for !cd.hasTargetBuildEventInfo() {
		select {
		case <-ctx.Done():
			return ctx.Err()
		default:
		}

		event := buildeventstream.BuildEvent{}
		if err := protodelim.UnmarshalFrom(reader, &event); err != nil {
			if errors.Is(err, io.EOF) {
				select {
				case <-ctx.Done():
					return ctx.Err()
				case <-timeout:
					return fmt.Errorf("timeout waiting for BES data")
				case <-time.After(50):
					// throttle the reading of the BES file when no new data is available
					continue
				}
			}

			return fmt.Errorf("failed to parse BES event: %w", err)
		}

		// We have received an event, reset the timer.
		timeout = time.After(timeoutd)

		switch event.Id.Id.(type) {
		case *buildeventstream.BuildEventId_ExecRequest:
			execPath := strings.Split(string(event.GetExecRequest().GetArgv()[2]), " ")[0]
			if cd.targetExecutablePath != "" && cd.targetExecutablePath != execPath {
				return fmt.Errorf("target executable path changed from %s to %s, this is not supported", cd.targetExecutablePath, execPath)
			}

			cd.targetExecutablePath = execPath

		case *buildeventstream.BuildEventId_NamedSet:
			// Record the named sets of files which the TargetCompleted event may reference.
			namedSets[event.Id.GetNamedSet().GetId()] = event.GetNamedSetOfFiles().GetFiles()

		case *buildeventstream.BuildEventId_TargetCompleted:
			if event.Id.GetTargetCompleted().Aspect != "" {
				continue
			}
			cd.targetLabel = event.Id.GetTargetCompleted().GetLabel()
			cd.targetTags = event.GetCompleted().GetTag()

			if importantOutput := event.GetCompleted().GetImportantOutput(); len(importantOutput) > 0 {
				// The deprecated "important output" path to the executable
				execPath := strings.TrimPrefix(importantOutput[0].GetUri(), "file://")
				if cd.targetExecutablePath != "" && cd.targetExecutablePath != execPath {
					return fmt.Errorf("target executable path changed from %s to %s, this is not supported", cd.targetExecutablePath, execPath)
				}

				cd.targetExecutablePath = execPath
			} else {
				// The default output group reference to the executable via named file sets
				for _, og := range event.GetCompleted().GetOutputGroup() {
					if og.Name == "default" {
						for _, f := range og.GetFileSets() {
							if files, hasGroup := namedSets[f.Id]; hasGroup && len(files) == 1 {
								execPath := path.Join(cd.workspaceDir, path.Join(files[0].PathPrefix...), files[0].Name)
								if cd.targetExecutablePath != "" && cd.targetExecutablePath != execPath {
									return fmt.Errorf("target executable path changed from %s to %s, this is not supported", cd.targetExecutablePath, execPath)
								}

								cd.targetExecutablePath = execPath
								break
							}
						}
						break
					}
				}
			}

		case *buildeventstream.BuildEventId_Workspace:
			cd.localExecroot = event.GetWorkspaceInfo().LocalExecRoot
		}

		// This is here to prevent deadlocking, when we reach the last BES
		// message, ideally should never happen, we stop the loop.
		if event.LastMessage {
			break
		}
	}

	if !cd.hasTargetBuildEventInfo() {
		return fmt.Errorf("failed to determine target information from build events: %v", cd.besFile.Name())
	}

	return nil
}

func (cd *ChangeDetector) explicitlySupportsIBP() bool {
	return slices.Contains(cd.targetTags, "supports_incremental_build_protocol")
}

func (cd *ChangeDetector) supportsIBazelNotifyChanges() bool {
	return slices.Contains(cd.targetTags, "ibazel_notify_changes")
}

func (cd *ChangeDetector) loadFullSourceInfo() (ibp.SourceInfoMap, error) {
	// Load the runfiles manifest to get the full list of files
	manifest, err := cd.parseRunfilesManifest()
	if err != nil {
		return nil, fmt.Errorf("failed to load runfiles manifest: %w", err)
	}

	sim := make(ibp.SourceInfoMap)

	for _, runfileInfo := range manifest.runfiles {
		sim[runfileInfo.runfilesPath] = &ibp.SourceInfo{
			IsSymlink: toJsonBoolPtr(runfileInfo.is_symlink),
			IsSource:  toJsonBoolPtr(runfileInfo.is_source),
		}
	}

	cd.sourcesInfo = sim
	cd.cycleSourceChanges = make(ibp.SourceInfoMap)

	return sim, nil
}

// Cycle reparses execution log to discover inputs
func (cd *ChangeDetector) detectChanges(sourceChanges []string) error {
	latestManifest, err := cd.parseRunfilesManifest()
	if err != nil {
		return fmt.Errorf("failed to cycle the runfiles manifest: %w", err)
	}
	execLogEntries, err := cd.cycleExecLog()
	if err != nil {
		return fmt.Errorf("failed to cycle the execlog: %w", err)
	}

	for _, execLogEntry := range execLogEntries {
		// The actual outputs are the files that were actually produced by the action
		if runfile, hasRunfile := latestManifest.fromInput(execLogEntry); hasRunfile {
			si := &ibp.SourceInfo{
				IsSymlink: toJsonBoolPtr(runfile.is_symlink),
				IsSource:  toJsonBoolPtr(runfile.is_source),
			}

			cd.cycleSourceChanges[runfile.runfilesPath] = si
			cd.sourcesInfo[runfile.runfilesPath] = si
		}
	}

	// Some source files may not be part of any action, but are still part of the runfiles tree.
	for _, changedSource := range sourceChanges {
		absSourcePath := path.Join(cd.localExecroot, changedSource)
		if runfile, hasRunfile := latestManifest.fromInput(absSourcePath); hasRunfile {
			si := &ibp.SourceInfo{
				IsSymlink: toJsonBoolPtr(runfile.is_symlink),
				IsSource:  toJsonBoolPtr(runfile.is_source),
			}

			cd.cycleSourceChanges[runfile.runfilesPath] = si
			cd.sourcesInfo[runfile.runfilesPath] = si
		}
	}

	// Deleted runfiles paths are those that were in the previous sources info but not the latest runfiles.
	for lastRunfilesPath := range cd.sourcesInfo {
		if _, stillHasRunfilesPath := latestManifest.runfiles[lastRunfilesPath]; !stillHasRunfilesPath {
			// Remove from the stored "last" source info
			delete(cd.sourcesInfo, lastRunfilesPath)

			// Mark as deleted in the "changed" source info
			cd.cycleSourceChanges[lastRunfilesPath] = nil
		}
	}

	return nil
}

func (cd *ChangeDetector) cycleChanges() ibp.SourceInfoMap {
	changed := cd.cycleSourceChanges
	cd.cycleSourceChanges = make(ibp.SourceInfoMap)
	return changed
}

// Cycle reparses execution log to discover inputs
func (cd *ChangeDetector) cycleExecLog() ([]string, error) {
	execLogFile, err := os.Open(cd.execlogFile.Name())
	if err != nil {
		return nil, err
	}
	defer execLogFile.Close()

	return parseExecLogInputs(execLogFile)
}

func parseExecLogInputs(in io.Reader) ([]string, error) {
	r := []string{}

	decode := json.NewDecoder(in)

	// collect the inputs
	for decode.More() {
		entry := ExecLogEntry{}
		if err := decode.Decode(&entry); err != nil {
			return nil, err
		}

		r = append(r, entry.ListedOutputs...)

		for _, output := range entry.Outputs {
			r = append(r, output.Path)
		}
	}

	return r, nil
}

// Cycle reparses execution log to discover inputs
func (cd *ChangeDetector) parseRunfilesManifest() (*manifestMetadata, error) {
	// TODO: cache based on manifest file stats?

	manifestPath := fmt.Sprintf("%s.runfiles_manifest", cd.targetExecutablePath)

	manifestFile, err := os.Open(manifestPath)
	if err != nil {
		return nil, err
	}
	defer manifestFile.Close()

	return parseRunfilesManifest(manifestFile, cd.workspaceDir, cd.localExecroot)
}

type manifestMetadata struct {
	runfilesOriginMapping map[string]string
	runfiles              map[string]*manifestEntry
}

type manifestEntry struct {
	runfilesPath string
	originPath   string
	is_external  bool
	is_symlink   bool
	is_source    bool
}

func parseRunfilesManifest(in io.Reader, sourceDir, localExecroot string) (*manifestMetadata, error) {
	entries := map[string]*manifestEntry{}
	bidi := map[string]string{}

	workspaceName := path.Base(localExecroot)
	workspaceNameSlash := workspaceName + "/"
	sourceDirSlash := sourceDir + "/"
	localExecrootSlash := localExecroot + "/"

	scan := bufio.NewScanner(in)

	// collect the inputs
	for scan.Scan() {
		line := scan.Text()
		sp := strings.SplitN(line, " ", 2)
		if len(sp) != 2 {
			return nil, fmt.Errorf("malformed runfiles manifest line: %s, %d", line, len(sp))
		}
		runfilesPath := sp[0]
		originPath := sp[1]

		is_external := false
		is_symlink := false
		is_source := false

		if !strings.HasPrefix(originPath, "/") {
			// Links are relative paths
			is_symlink = true
		} else if strings.HasPrefix(originPath, sourceDirSlash) {
			// Sources are still in their original location, not copied into the runfiles or bindir
			is_source = true

			originPath = originPath[len(sourceDirSlash):]
		} else if strings.HasPrefix(originPath, localExecrootSlash) {
			// Generated files are in the local execroot
			originPath = originPath[len(localExecrootSlash):]
		} else if !strings.HasPrefix(runfilesPath, workspaceNameSlash) {
			// External files have a runfiles path without the main workspace name.
			is_external = true
		}

		// Generated and source files may be looked-up by their original path
		if !is_symlink && !is_external {
			bidi[originPath] = runfilesPath
		}

		entries[runfilesPath] = &manifestEntry{
			runfilesPath: runfilesPath,
			originPath:   originPath,
			is_external:  is_external,
			is_symlink:   is_symlink,
			is_source:    is_source,
		}
	}

	return &manifestMetadata{runfiles: entries, runfilesOriginMapping: bidi}, nil
}

func (m *manifestMetadata) fromInput(f string) (*manifestEntry, bool) {
	runfile, ok := m.runfilesOriginMapping[f]
	if !ok {
		return nil, false
	}
	return m.runfiles[runfile], ok
}

func toJsonBoolPtr(b bool) *bool {
	if b {
		return &b
	}
	return nil
}
