/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package bazel

import (
	"encoding/base64"
	"fmt"
	"io"
	"io/ioutil"

	"github.com/bazelbuild/bazelisk/core"
	"github.com/bazelbuild/bazelisk/repositories"
	"google.golang.org/protobuf/proto"
)

type Bazel interface {
	SetWorkspaceRoot(workspaceRoot string)
	Spawn(command []string) (int, error)
	RunCommand(command []string, out io.Writer) (int, error)
}

type bazel struct {
	workspaceRoot string
}

func New() Bazel {
	return &bazel{}
}

func (b *bazel) SetWorkspaceRoot(workspaceRoot string) {
	b.workspaceRoot = workspaceRoot
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
	if len(b.workspaceRoot) < 1 {
		panic("Illegal state: running bazel without the workspaceRoot set")
	}

	bazelisk := NewBazelisk(b.workspaceRoot)
	exitCode, err := bazelisk.Run(command, repos, out)
	return exitCode, err
}

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
