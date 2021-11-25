/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package pathutils

import (
	"io/fs"
	"path"
	"path/filepath"
	"testing"
	"time"

	. "github.com/onsi/gomega"
	"github.com/spf13/cobra"
)

// FakeFile implements fs.FileInfo
// Adapted from https://go.dev/src/cmd/pack/pack_test.go
type FakeFile struct {
	name string
}

func (f *FakeFile) Name() string {
	// A bit of a cheat: we only have a basename, so that's also ok for FileInfo.
	return f.name
}

func (f *FakeFile) Size() int64 {
	return int64(len(""))
}

func (f *FakeFile) Mode() fs.FileMode {
	return 0644
}

func (f *FakeFile) ModTime() time.Time {
	return time.Time{}
}

func (f *FakeFile) IsDir() bool {
	return false
}

func (f *FakeFile) Sys() interface{} {
	return nil
}

func getMockCmd(osGetwd func() (string, error)) *cobra.Command {
	return &cobra.Command{
		Use:   "mock",
		Short: "",
		Long:  "",
		Args:  cobra.MaximumNArgs(1),
		RunE: invokeCmdInsideWorkspace(
			osGetwd,
			defaultWorkspaceFinder,
			func(cmd *cobra.Command, args []string) error {
				return nil
			},
		),
	}
}

func TestWorkspaceFinder(t *testing.T) {
	t.Run("find succeeds when cwd is a path inside Bazel workspace", func(t *testing.T) {
		g := NewGomegaWithT(t)

		for _, workspaceFilename := range WorkspaceFilenames {

			// mock osStat to always return a FakeFile for each workspace file name
			// (since Find also loops over the workspace file names, we have to make sure that
			// the iterators match)
			osStat := func(filePath string) (fs.FileInfo, error) {
				curWorkspaceFilename := filepath.Base(filePath)
				if curWorkspaceFilename == workspaceFilename {
					return &FakeFile{
						name: filepath.Base(filePath),
					}, nil
				}
				return nil, fs.ErrNotExist
			}

			workspaceFinder := &WorkspaceFinder{osStat: osStat}
			workspacePath, err := workspaceFinder.Find("test")
			g.Expect(err).To(BeNil())
			g.Expect(workspacePath).To(Equal(path.Join("test", workspaceFilename)))
		}
	})
	t.Run("find fails when cwd is a path outside Bazel workspace", func(t *testing.T) {
		g := NewGomegaWithT(t)

		// mock osStat to always return a fs.ErrNotExist => no parent of the cwd contains a WORKSPACE file
		osStat := func(string) (fs.FileInfo, error) {
			return nil, fs.ErrNotExist
		}

		workspaceFinder := &WorkspaceFinder{osStat: osStat}
		workspacePath, err := workspaceFinder.Find(filepath.Join("test", "foo", "bar", "baz"))
		g.Expect(err).To(BeNil())
		g.Expect(workspacePath).To(Equal(""))
	})
	t.Run("find fails when cwd is an empty relative path", func(t *testing.T) {
		g := NewGomegaWithT(t)

		// we don't care about osStat here since it won't be called, so just return nil, nil
		osStat := func(string) (fs.FileInfo, error) {
			return nil, nil
		}

		workspaceFinder := &WorkspaceFinder{osStat: osStat}
		workspacePath, err := workspaceFinder.Find(".")
		g.Expect(err).To(BeNil())
		g.Expect(workspacePath).To(Equal(""))
	})
	t.Run("find fails when cwd is the absolute root", func(t *testing.T) {
		g := NewGomegaWithT(t)

		// we don't care about osStat here since it won't be called, so just return nil, nil
		osStat := func(string) (fs.FileInfo, error) {
			return nil, nil
		}

		workspaceFinder := &WorkspaceFinder{osStat: osStat}
		workspacePath, err := workspaceFinder.Find(string(filepath.Separator))
		g.Expect(err).To(BeNil())
		g.Expect(workspacePath).To(Equal(""))
	})
}

func TestInvokeCmdInsideWorkspace(t *testing.T) {

	t.Run("invoke command inside a workspace", func(t *testing.T) {
		g := NewGomegaWithT(t)

		osGetwd := func() (string, error) {
			return "testfixtures/workspace_1", nil
		}

		// invoke mock command
		err := getMockCmd(osGetwd).Execute()
		g.Expect(err).To(BeNil())
	})

	t.Run("invoke command outside workspace", func(t *testing.T) {
		g := NewGomegaWithT(t)

		osGetwd := func() (string, error) {
			return "testfixtures/", nil
		}

		// invoke mock command
		err := getMockCmd(osGetwd).Execute()
		g.Expect(err.Error()).To(Equal(
			"failed to run command \"mock\" inside workspace: " +
				"the current working directory \"testfixtures/\" is not a bazel workspace " +
				"(below a directory having a WORKSPACE file)"))
	})
}
