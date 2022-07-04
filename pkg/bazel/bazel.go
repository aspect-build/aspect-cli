/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package bazel

import (
	"aspect.build/cli/pkg/ioutils"
	"encoding/base64"
	"fmt"
	"io"
	"io/ioutil"
	"os"
	"path"
	"strings"

	"aspect.build/cli/bazel/analysis"
	"aspect.build/cli/bazel/flags"
	"aspect.build/cli/pkg/pathutils"

	"github.com/bazelbuild/bazelisk/core"
	"github.com/bazelbuild/bazelisk/repositories"
	"google.golang.org/protobuf/proto"
)

// Global mutable state!
// This is for performance, avoiding a lookup of the workspace directory for every
// instance of a bazel struct.
// We know the workspace location is constant for the lifetime of an `aspect` cli execution.
var defaultWorkspaceRoot string

// Global mutable state!
// This is for performance, avoiding a lookup of the possible startup flags for every
// instance of a bazel struct.
// We know the flags will be constant for the lifetime of an `aspect` cli execution.
var availableStartupFlags []string

// Global mutable state!
// This is for performance, avoiding needing to set the specified startup flags for every
// instance of a bazel struct.
// We know the specified startup flags will be constant for the lifetime of an `aspect`
// cli execution.
var startupFlags []string

type BazelContext struct {
	WorkspaceRoot string
	EnvVars       map[string]string
	Streams       ioutils.Streams
}

type Bazel interface {
	AQuery(expr string) (*analysis.ActionGraphContainer, error)
	Spawn(command ...string) (int, error)
	RunCommand(context BazelContext, command ...string) (int, error)
	Flags() (map[string]*flags.FlagInfo, error)
	AvailableStartupFlags() []string
	SetStartupFlags(flags []string)
}

type bazel struct {
	osGetwd         func() (dir string, err error)
	workspaceFinder pathutils.WorkspaceFinder
}

func New() Bazel {
	return &bazel{
		osGetwd:         os.Getwd,
		workspaceFinder: pathutils.DefaultWorkspaceFinder,
	}
}

func DefaultBazelContext() BazelContext {
	return BazelContext{
		WorkspaceRoot: "",
		EnvVars:       nil,
		Streams:       ioutils.DefaultStreams,
	}
}

// maybeSetWorkspaceRoot lazily sets the defaultWorkspaceRoot if it isn't set already.
func (b *bazel) maybeSetWorkspaceRoot() error {
	fail := func(err error) error {
		return fmt.Errorf("failed to find bazel workspace root: %w", err)
	}
	if len(defaultWorkspaceRoot) < 1 {
		wd, err := b.osGetwd()
		if err != nil {
			return fail(err)
		}
		workspacePath, err := b.workspaceFinder.Find(wd)
		if err != nil {
			return fail(err)
		}
		if workspacePath == "" {
			return fail(fmt.Errorf("the current working directory %q is not a Bazel workspace", wd))
		}
		defaultWorkspaceRoot = path.Dir(workspacePath)
	}
	return nil
}

// AvailableStartupFlags will return the full list of startup flags available for
// the current version of bazel. This is NOT the list of startup flags that have been
// set for the current run via SetStartupFlags.
func (b *bazel) AvailableStartupFlags() []string {
	if len(availableStartupFlags) == 0 {
		b.Flags()
	}
	return availableStartupFlags
}

// SetStartupFlags will set the startup flags to be used by bazel during all bazel runs
// performed during the current instantiation of the aspect CLI.
func (b *bazel) SetStartupFlags(flags []string) {
	startupFlags = flags
}

func (*bazel) createRepositories() *core.Repositories {
	gcs := &repositories.GCSRepo{}
	gitHub := repositories.CreateGitHubRepo(core.GetEnvOrConfig("BAZELISK_GITHUB_TOKEN"))
	// Fetch LTS releases, release candidates and Bazel-at-commits from GCS, forks and rolling releases from GitHub.
	// TODO(https://github.com/bazelbuild/bazelisk/issues/228): get rolling releases from GCS, too.
	return core.CreateRepositories(gcs, gcs, gitHub, gcs, gitHub, true)
}

// Spawn is similar to the main() function of bazelisk
// see https://github.com/bazelbuild/bazelisk/blob/7c3d9d5/bazelisk.go
func (b *bazel) Spawn(command ...string) (int, error) {
	return b.RunCommand(DefaultBazelContext(), command...)
}

func (b *bazel) RunCommand(context BazelContext, command ...string) (int, error) {
	// Prepend startup flags
	command = append(startupFlags, command...)

	repos := b.createRepositories()

	var workspaceRoot string
	if context.WorkspaceRoot == "" {
		if err := b.maybeSetWorkspaceRoot(); err != nil {
			return 1, err
		}
		workspaceRoot = defaultWorkspaceRoot
	} else {
		workspaceRoot = context.WorkspaceRoot
	}

	bazelisk := NewBazelisk(workspaceRoot)
	exitCode, err := bazelisk.Run(command, repos, context.Streams)
	return exitCode, err
}

// Flags fetches the metadata for Bazel's command line flags.
func (b *bazel) Flags() (map[string]*flags.FlagInfo, error) {
	r, w := io.Pipe()
	streams := ioutils.Streams{
		Stdin:  os.Stdin,
		Stdout: w,
		Stderr: nil,
	}
	decoder := base64.NewDecoder(base64.StdEncoding, r)
	bazelErrs := make(chan error, 1)
	defer close(bazelErrs)
	go func() {
		defer w.Close()
		_, err := b.RunCommand(
			BazelContext{
				WorkspaceRoot: "",
				EnvVars:       nil,
				Streams:       streams,
			},
			"help", "flags-as-proto")
		bazelErrs <- err
	}()

	helpProtoBytes, err := ioutil.ReadAll(decoder)
	if err != nil {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", err)
	}

	if err := <-bazelErrs; err != nil {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", err)
	}

	flagCollection := &flags.FlagCollection{}
	if err := proto.Unmarshal(helpProtoBytes, flagCollection); err != nil {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", err)
	}

	flags := make(map[string]*flags.FlagInfo)
	for i := range flagCollection.FlagInfos {
		flags[*flagCollection.FlagInfos[i].Name] = flagCollection.FlagInfos[i]
		for _, command := range flags[*flagCollection.FlagInfos[i].Name].Commands {
			if command == "startup" {
				availableStartupFlags = append(availableStartupFlags, *flagCollection.FlagInfos[i].Name)
				if flags[*flagCollection.FlagInfos[i].Name].GetHasNegativeFlag() {
					availableStartupFlags = append(availableStartupFlags, "no"+*flagCollection.FlagInfos[i].Name)
				}
			}
		}
	}

	return flags, nil
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
		_, err := b.RunCommand(
			BazelContext{
				WorkspaceRoot: "",
				EnvVars:       nil,
				Streams:       streams,
			},
			"aquery", "--output=proto")
		bazelErrs <- err
	}()

	protoBytes, err := ioutil.ReadAll(r)
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

type Output struct {
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

	result := make([]Output, 10)
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
				a.Mnemonic,
				path.String(),
			})
		}
	}
	return result
}
