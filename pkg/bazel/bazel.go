/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package bazel

import (
	"encoding/base64"
	"fmt"
	"io"
	"io/ioutil"
	"os"
	"path"
	"strings"

	"aspect.build/cli/pkg/pathutils"

	"github.com/bazelbuild/bazelisk/core"
	"github.com/bazelbuild/bazelisk/repositories"
	"google.golang.org/protobuf/proto"
)

// This is global so that if we can have multiple implementations of bazel
// without needing to either find / set the workspace root every time.
// Will be set when the first instance of bazel is created via "New()".
var workspaceRoot string = ""

type Bazel interface {
	AQuery(expr string) (*ActionGraphContainer, error)
	SetWorkspaceRoot(workspaceRoot string)
	Spawn(command []string) (int, error)
	RunCommand(command []string, out io.Writer) (int, error)
	Flags() (map[string]*FlagInfo, error)
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

// Deprecated. WorkspaceRoot is set lazily by this class
func (b *bazel) SetWorkspaceRoot(w string) {
	workspaceRoot = w
}

// maybeSetWorkspaceRoot lazily sets the workspaceRoot if it isn't set already.
func (b *bazel) maybeSetWorkspaceRoot() error {
	fail := func(err error) error {
		return fmt.Errorf("failed to find bazel workspace root: %w", err)
	}
	if len(workspaceRoot) < 1 {
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
		workspaceRoot = path.Dir(workspacePath)
	}
	return nil
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
func (b *bazel) Spawn(command []string) (int, error) {
	return b.RunCommand(command, nil)
}

func (b *bazel) RunCommand(command []string, out io.Writer) (int, error) {
	repos := b.createRepositories()
	if err := b.maybeSetWorkspaceRoot(); err != nil {
		return 1, err
	}

	bazelisk := NewBazelisk(workspaceRoot)
	exitCode, err := bazelisk.Run(command, repos, out)
	return exitCode, err
}

// Flags fetches the metadata for Bazel's command line flags.
func (b *bazel) Flags() (map[string]*FlagInfo, error) {
	r, w := io.Pipe()
	decoder := base64.NewDecoder(base64.StdEncoding, r)
	bazelErrs := make(chan error, 1)
	defer close(bazelErrs)
	go func() {
		defer w.Close()
		_, err := b.RunCommand([]string{"help", "flags-as-proto"}, w)
		bazelErrs <- err
	}()

	helpProtoBytes, err := ioutil.ReadAll(decoder)
	if err != nil {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", err)
	}

	if err := <-bazelErrs; err != nil {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", err)
	}

	flagCollection := &FlagCollection{}
	if err := proto.Unmarshal(helpProtoBytes, flagCollection); err != nil {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", err)
	}

	flags := make(map[string]*FlagInfo)
	for i := range flagCollection.FlagInfos {
		flags[*flagCollection.FlagInfos[i].Name] = flagCollection.FlagInfos[i]
	}

	return flags, nil
}

// AQuery runs a `bazel aquery` command and returns the resulting parsed proto data.
func (b *bazel) AQuery(query string) (*ActionGraphContainer, error) {
	r, w := io.Pipe()
	agc := &ActionGraphContainer{}

	bazelErrs := make(chan error, 1)
	defer close(bazelErrs)
	go func() {
		defer w.Close()
		_, err := b.RunCommand([]string{"aquery", "--output=proto", query}, w)
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
func ParseOutputs(agc *ActionGraphContainer) []Output {
	// Use RAM to store lookup maps for these identifiers
	// rather than an O(n^2) algorithm of searching on each access.
	frags := make(map[uint32]*PathFragment)
	for _, f := range agc.PathFragments {
		frags[f.Id] = f
	}
	arts := make(map[uint32]*Artifact)
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
			fmt.Println(a.Mnemonic)
			result = append(result, Output{
				a.Mnemonic,
				path.String(),
			})
		}
	}
	return result
}
