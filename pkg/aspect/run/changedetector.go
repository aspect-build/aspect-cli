package run

import (
	"bufio"
	"crypto/sha256"
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path"
	"slices"
	"strings"

	"github.com/aspect-build/orion/common/ibp"
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
	workspaceDir      string
	execlogFile       *os.File
	watchManifestFile *os.File

	// Current known state of sources
	sourcesInfo ibp.SourceInfoMap

	watchRepoDir  string
	watchRepoName string

	// Changes detected to the sources since the last cycleChanges() call.
	cycleSourceChanges ibp.SourceInfoMap

	targetTags           []string
	targetLabel          string
	targetExecutablePath string
	localExecroot        string

	// Support bazel <8
	useLegacyReplaceWorkspace bool
}

//go:embed aspect_watch.bzl
var ASPECT_WATCH_BZL_CONTENT []byte

func newChangeDetector(workspaceDir string, useLegacyReplaceWorkspace bool) (*ChangeDetector, error) {
	execlog, err := os.CreateTemp(os.TempDir(), fmt.Sprintf("aspect-watch-%v-execlog-*.json", os.Getpid()))
	if err != nil {
		return nil, err
	}

	watchManifest, err := os.CreateTemp(os.TempDir(), fmt.Sprintf("aspect-watch-%v-*.manifest", os.Getpid()))
	if err != nil {
		return nil, err
	}

	watchRepoName, watchRepoDir, err := createUserWatchRepo()
	if err != nil {
		return nil, err
	}

	return &ChangeDetector{
		workspaceDir:         workspaceDir,
		execlogFile:          execlog,
		watchManifestFile:    watchManifest,
		watchRepoDir:         watchRepoDir,
		watchRepoName:        watchRepoName,
		targetTags:           []string{},
		targetLabel:          "",
		targetExecutablePath: "",

		useLegacyReplaceWorkspace: useLegacyReplaceWorkspace,
	}, nil
}

func (cd *ChangeDetector) Close() error {
	return errors.Join(
		cd.execlogFile.Close(),
		cd.watchManifestFile.Close(),
		os.Remove(cd.execlogFile.Name()),
		os.Remove(cd.watchManifestFile.Name()),
	)
}

func (cd *ChangeDetector) bazelFlags(trackChanges bool) []string {
	flags := []string{}

	if trackChanges {
		// TODO: maybe use a more compact format for better performance?
		flags = append(flags, "--execution_log_json_file", cd.execlogFile.Name(), "--noexecution_log_sort")
	}

	injectArgName := "inject_repository"
	aspectRepoPrefix := "@"
	if cd.useLegacyReplaceWorkspace {
		injectArgName = "override_repository"
		aspectRepoPrefix = "@@"
	}

	flags = append(flags, fmt.Sprintf("--%s=%s=%s", injectArgName, cd.watchRepoName, cd.watchRepoDir))
	flags = append(flags, fmt.Sprintf("--aspects=%s%s//:aspect_watch.bzl%%watch_manifest", aspectRepoPrefix, cd.watchRepoName))
	flags = append(flags, "--output_groups=+__aspect_watch_watch_manifest", fmt.Sprintf("--aspects_parameters=aspect_watch_watch_manifest=%s", cd.watchManifestFile.Name()))

	return flags
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

// Detect initial context for the run target.
func (cd *ChangeDetector) detectContext() error {
	manifestFile, err := os.ReadFile(cd.watchManifestFile.Name())
	if err != nil {
		return fmt.Errorf("failed to read watch manifest file: %w", err)
	}
	lines := strings.Split(string(manifestFile), "\n")
	if len(lines) != 5 || lines[4] != "" {
		return fmt.Errorf("watch manifest file (%s) is malformed, expected 5 lines, got %d:\n%s", cd.watchManifestFile.Name(), len(lines), strings.Join(lines, "\n"))
	}

	cd.localExecroot = lines[0]
	cd.targetExecutablePath = lines[1]
	cd.targetLabel = lines[2]
	cd.targetTags = strings.Split(lines[3], ",")

	return nil
}

// Detect changes after an incremental triggered by the source changes.
func (cd *ChangeDetector) detectChanges(sourceChanges []string) error {
	err := cd.detectContext()
	if err != nil {
		return fmt.Errorf("failed to detect context: %w", err)
	}

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

	if cd.targetExecutablePath == "" {
		return nil, fmt.Errorf("targetExecutablePath is not set")
	}

	manifestPath := fmt.Sprintf("%s.runfiles_manifest", cd.targetExecutablePath)

	manifestFile, err := os.Open(path.Join(cd.localExecroot, manifestPath))
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

func createUserWatchRepo() (string, string, error) {
	cacheDir, err := os.UserCacheDir()
	if err != nil {
		return "", "", err
	}

	name := fmt.Sprintf("aspect-watch-%x", sha256.Sum256(ASPECT_WATCH_BZL_CONTENT))
	dir := path.Join(cacheDir, name)

	// If the directory already exists simply return it. The sha256 hash ensures that the
	// repo is up to date and contains the correct files.
	if d, err := os.Stat(dir); err == nil && d.IsDir() {
		return name, dir, nil
	}

	if err := os.MkdirAll(dir, 0755); err != nil {
		return name, dir, err
	}
	if err := os.WriteFile(path.Join(dir, "aspect_watch.bzl"), ASPECT_WATCH_BZL_CONTENT, 0644); err != nil {
		return name, dir, err
	}
	if err := os.WriteFile(path.Join(dir, "MODULE.bazel"), []byte{}, 0644); err != nil {
		return name, dir, err
	}
	if err := os.WriteFile(path.Join(dir, "BUILD.bazel"), []byte{}, 0644); err != nil {
		return name, dir, err
	}
	if err := os.WriteFile(path.Join(dir, "WORKSPACE"), []byte{}, 0644); err != nil {
		return name, dir, err
	}
	return name, dir, nil
}
