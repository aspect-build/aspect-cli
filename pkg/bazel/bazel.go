/*
 * Copyright 2022 Aspect Build Systems, Inc.
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

package bazel

import (
	"bytes"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"aspect.build/cli/bazel/analysis"
	"aspect.build/cli/bazel/flags"
	"aspect.build/cli/pkg/bazel/workspace"
	"aspect.build/cli/pkg/ioutils"
	"github.com/spf13/cobra"

	"github.com/bazelbuild/bazelisk/core"
	"github.com/bazelbuild/bazelisk/repositories"
	"google.golang.org/protobuf/proto"
)

// Global mutable state!
// This is for performance, avoiding a lookup of the possible startup flags for every
// instance of a bazel struct.
// We know the flags will be constant for the lifetime of an `aspect` cli execution.
var allFlags map[string]*flags.FlagInfo

// Global mutable state!
// This is for performance, avoiding needing to set the specified startup flags for every
// instance of a bazel struct.
// We know the specified startup flags will be constant for the lifetime of an `aspect`
// cli execution.
var startupFlags []string

type BazelProvider func() (Bazel, error)

type Bazel interface {
	WithEnv(env []string) Bazel
	AQuery(expr string) (*analysis.ActionGraphContainer, error)
	MaybeReenterAspect(streams ioutils.Streams, args []string) (bool, int, error)
	RunCommand(streams ioutils.Streams, wd *string, command ...string) (int, error)
	InitializeBazelFlags() error
	Flags() (map[string]*flags.FlagInfo, error)
	AbsPathRelativeToWorkspace(relativePath string) (string, error)
	AddBazelFlags(cmd *cobra.Command) error
}

type bazel struct {
	workspaceRoot string
	env           []string
}

func New(workspaceRoot string) (Bazel, error) {
	// If we are given a non-empty workspace root, make sure that it is an absolute path. We support
	// an empty workspace root for Bazel commands that support being run outside of a workspace
	// (e.g. version).
	absWkspRoot, err := filepath.Abs(workspaceRoot)
	if err != nil {
		return nil, err
	}
	return &bazel{
		workspaceRoot: absWkspRoot,
	}, nil
}

func NoWorkspaceRoot() Bazel {
	// This is a special case where we run Bazel without a workspace (e.g., version).
	return &bazel{}
}

func Find(startDir string) (Bazel, error) {
	finder := workspace.DefaultFinder
	wr, err := finder.Find(startDir)
	if err != nil {
		return NoWorkspaceRoot(), nil
	}
	return New(wr)
}

func FindFromWd() (Bazel, error) {
	wd, err := os.Getwd()
	if err != nil {
		return nil, err
	}
	return Find(wd)
}

func (b *bazel) WithEnv(env []string) Bazel {
	b.env = env
	return b
}

func (*bazel) createRepositories() *core.Repositories {
	gcs := &repositories.GCSRepo{}
	gitHub := repositories.CreateGitHubRepo(core.GetEnvOrConfig("BAZELISK_GITHUB_TOKEN"))
	// Fetch LTS releases, release candidates and Bazel-at-commits from GCS, forks and rolling releases from GitHub.
	// TODO(https://github.com/bazelbuild/bazelisk/issues/228): get rolling releases from GCS, too.
	return core.CreateRepositories(gcs, gcs, gitHub, gcs, gcs, true)
}

// Check if we should re-enter a different version and/or tier of the Aspect CLI and re-enter if we should.
// Error is returned if version and/or tier are misconfigured in the Aspect CLI config.
func (b *bazel) MaybeReenterAspect(streams ioutils.Streams, args []string) (bool, int, error) {
	bazelisk := NewBazelisk(b.workspaceRoot)

	// Calling bazelisk.getBazelVersion() has the side-effect of setting AspectShouldReenter.
	// TODO: this pattern could get cleaned up so it does not rely on the side-effect
	bazelisk.getBazelVersion()

	if bazelisk.AspectShouldReenter {
		repos := b.createRepositories()
		exitCode, err := bazelisk.Run(args, repos, streams, b.env, nil)
		return true, exitCode, err
	}

	return false, 0, nil
}

func (b *bazel) RunCommand(streams ioutils.Streams, wd *string, command ...string) (int, error) {
	// Prepend startup flags
	command = append(startupFlags, command...)

	repos := b.createRepositories()

	bazelisk := NewBazelisk(b.workspaceRoot)

	exitCode, err := bazelisk.Run(command, repos, streams, b.env, wd)
	return exitCode, err
}

// Initializes start-up flags from args and returns args without start-up flags
func InitializeStartupFlags(args []string) ([]string, error) {
	nonFlags, flags, err := ParseOutBazelFlags("startup", args)
	if err != nil {
		return nil, err
	}

	startupFlags = flags
	return nonFlags, nil
}

// Flags fetches the metadata for Bazel's command line flag via `bazel help flags-as-proto`
func (b *bazel) Flags() (map[string]*flags.FlagInfo, error) {
	if allFlags != nil {
		return allFlags, nil
	}
	var buffer bytes.Buffer
	streams := ioutils.Streams{
		Stdin:  os.Stdin,
		Stdout: &buffer,
		Stderr: nil,
	}
	decoder := base64.NewDecoder(base64.StdEncoding, &buffer)
	bazelErrs := make(chan error, 1)
	bazelExitCode := make(chan int, 1)
	defer close(bazelErrs)
	defer close(bazelExitCode)
	go func() {
		// Running in batch mode will prevent bazel from spawning a daemon. Spawning a bazel daemon takes time which is something we don't want here.
		// Also, instructing bazel to ignore all rc files will protect it from failing if any of the rc files is broken.
		tmpdir := os.TempDir()
		exitCode, err := b.RunCommand(streams, &tmpdir, "--nobatch", "--ignore_all_rc_files", "help", "flags-as-proto")
		bazelErrs <- err
		bazelExitCode <- exitCode
	}()

	if exitCode := <-bazelExitCode; exitCode != 0 {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", fmt.Errorf("bazel has quit with code %d", exitCode))
	}

	if err := <-bazelErrs; err != nil {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", err)
	}

	helpProtoBytes, err := io.ReadAll(decoder)
	if err != nil {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", err)
	}

	flagCollection := &flags.FlagCollection{}
	if err := proto.Unmarshal(helpProtoBytes, flagCollection); err != nil {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", err)
	}

	allFlags = make(map[string]*flags.FlagInfo)

	for i := range flagCollection.FlagInfos {
		allFlags[*flagCollection.FlagInfos[i].Name] = flagCollection.FlagInfos[i]
	}

	return allFlags, nil
}

// AQuery runs a `bazel aquery` command and returns the resulting parsed proto data.
func (b *bazel) AQuery(query string) (*analysis.ActionGraphContainer, error) {
	r, w := io.Pipe()
	streams := ioutils.Streams{
		Stdin:  os.Stdin,
		Stdout: w,
		Stderr: nil,
	}
	agc := &analysis.ActionGraphContainer{}

	bazelErrs := make(chan error, 1)
	defer close(bazelErrs)
	go func() {
		defer w.Close()
		_, err := b.RunCommand(streams, nil, "aquery", "--output=proto", query)
		bazelErrs <- err
	}()

	protoBytes, err := io.ReadAll(r)
	if err != nil {
		return nil, fmt.Errorf("failed to run Bazel aquery: %w", err)
	}

	if err := <-bazelErrs; err != nil {
		return nil, fmt.Errorf("failed to run Bazel aquery: %w", err)
	}

	proto.Unmarshal(protoBytes, agc)
	if err := proto.Unmarshal(protoBytes, agc); err != nil {
		return nil, fmt.Errorf("failed to run Bazel aquery: parsing ActionGraphContainer: %w", err)
	}
	return agc, nil
}

func (b *bazel) AbsPathRelativeToWorkspace(relativePath string) (string, error) {
	if b.workspaceRoot == "" {
		return "", errors.New("the bazel instance does not have a workspace root")
	}
	if filepath.IsAbs(relativePath) {
		return relativePath, nil
	}
	return filepath.Join(b.workspaceRoot, relativePath), nil
}

type Output struct {
	Label    string
	Mnemonic string
	Path     string
}

// ParseOutputs reads the proto result of AQuery and extracts the output file paths with their generator mnemonics.
func ParseOutputs(agc *analysis.ActionGraphContainer) []Output {
	// Use RAM to store lookup maps for these identifiers
	// rather than an O(n^2) algorithm of searching on each access.
	frags := make(map[uint32]*analysis.PathFragment)
	for _, f := range agc.PathFragments {
		frags[f.Id] = f
	}
	arts := make(map[uint32]*analysis.Artifact)
	for _, a := range agc.Artifacts {
		arts[a.Id] = a
	}
	targets := make(map[uint32]*analysis.Target)
	for _, t := range agc.Targets {
		targets[t.Id] = t
	}

	// The paths in the proto data are organized as a trie
	// to make the representation more compact.
	// https://en.wikipedia.org/wiki/Trie
	// Make a map to store each prefix so we can memoize common paths
	prefixes := make(map[uint32]*[]string)

	// Declare a recursive function to walk up the trie to the root.
	var prefix func(pathID uint32) []string

	prefix = func(pathID uint32) []string {
		if prefixes[pathID] != nil {
			return *prefixes[pathID]
		}
		fragment := frags[pathID]
		// Reconstruct the path from the parent pointers.
		segments := []string{fragment.Label}

		if fragment.ParentId > 0 {
			segments = append(segments, prefix(fragment.ParentId)...)
		}
		prefixes[pathID] = &segments
		return segments
	}

	var result []Output
	for _, a := range agc.Actions {
		for _, i := range a.OutputIds {
			artifact := arts[i]
			segments := prefix(artifact.PathFragmentId)
			var path strings.Builder
			// Assemble in reverse order.
			for i := len(segments) - 1; i >= 0; i-- {
				path.WriteString(segments[i])
				if i > 0 {
					path.WriteString("/")
				}
			}
			result = append(result, Output{
				targets[a.TargetId].Label,
				a.Mnemonic,
				path.String(),
			})
		}
	}
	return result
}
