/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package bazel

import (
	"github.com/bazelbuild/bazelisk/core"
	"github.com/bazelbuild/bazelisk/repositories"
	"io"
)

type Spawner interface {
	Spawn(command []string) (int, error)
}

type Bazel struct {
}

func New() *Bazel {
	return &Bazel{}
}

func (*Bazel) createRepositories() *core.Repositories {
	gcs := &repositories.GCSRepo{}
	gitHub := repositories.CreateGitHubRepo(core.GetEnvOrConfig("BAZELISK_GITHUB_TOKEN"))
	// Fetch LTS releases, release candidates and Bazel-at-commits from GCS, forks and rolling releases from GitHub.
	// TODO(https://github.com/bazelbuild/bazelisk/issues/228): get rolling releases from GCS, too.
	return core.CreateRepositories(gcs, gcs, gitHub, gcs, gitHub, true)
}

// Spawn is similar to the main() function of bazelisk
// see https://github.com/bazelbuild/bazelisk/blob/7c3d9d5/bazelisk.go
func (bazel *Bazel) Spawn(command []string) (int, error) {
	return bazel.RunCommand(command, nil)
}

func (bazel *Bazel) RunCommand(command []string, out io.Writer) (int, error) {
	repos := bazel.createRepositories()
	exitCode, err := RunBazelisk(command, repos, out)
	return exitCode, err
}
