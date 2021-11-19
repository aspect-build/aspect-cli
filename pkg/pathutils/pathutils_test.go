/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package pathutils_test

import (
	"fmt"
	"os"
	"path/filepath"
	"testing"

	. "github.com/onsi/gomega"

	"aspect.build/cli/pkg/aspect/info"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/pathutils"
)

func TestIsFile(t *testing.T) {

	t.Run("path is file", func(t *testing.T) {
		g := NewGomegaWithT(t)
		path := "testfixtures/test.txt"
		isFile := pathutils.IsFile(path)
		g.Expect(isFile).To(BeTrue())
	})

	t.Run("path is directory", func(t *testing.T) {
		g := NewGomegaWithT(t)
		path := "testfixtures"
		isFile := pathutils.IsFile(path)
		g.Expect(isFile).To(BeFalse())
	})

	t.Run("path is invalid", func(t *testing.T) {
		g := NewGomegaWithT(t)
		path := "test"
		isFile := pathutils.IsFile(path)
		g.Expect(isFile).To(BeFalse())
	})
}

func TestIsValidWorkspace(t *testing.T) {

	t.Run("path is a workspace root", func(t *testing.T) {
		t.Run("with WORKSPACE file", func(t *testing.T) {
			g := NewGomegaWithT(t)
			path := "testfixtures/workspace_1"
			isWorkspace := pathutils.IsWorkspace(path)
			g.Expect(isWorkspace).To(BeTrue())
		})

		t.Run("with WORKSPACE.bazel file", func(t *testing.T) {
			g := NewGomegaWithT(t)
			path := "testfixtures/workspace_2"
			isWorkspace := pathutils.IsWorkspace(path)
			g.Expect(isWorkspace).To(BeTrue())
		})
	})

	t.Run("path is not a workspace root", func(t *testing.T) {
		g := NewGomegaWithT(t)
		path := "testfixtures"
		isWorkspace := pathutils.IsWorkspace(path)
		g.Expect(isWorkspace).To(BeFalse())
	})
}

func TestIsValidPackage(t *testing.T) {

	t.Run("path is a package", func(t *testing.T) {
		t.Run("with BUILD file", func(t *testing.T) {
			g := NewGomegaWithT(t)
			path := "testfixtures/workspace_1/pkg_1"
			// We have to rename BUILD_bazel to BUILD.bazel because we can't include
			// a file named BUILD.bazel as part of the go_test data (labels cannot cross
			// package boundaries)
			err := os.Rename(path + "/BUILD_bazel", path + "/BUILD.bazel")
			g.Expect(err).To(BeNil())

			isPkg := pathutils.IsPackage(path)
			g.Expect(isPkg).To(BeTrue())
		})

		t.Run("with BUILD.bazel file", func(t *testing.T) {
			g := NewGomegaWithT(t)
			path := "testfixtures/workspace_1/pkg_2"
			// We have to rename BUILD_ to BUILD because we can't include
			// a file named BUILD as part of the go_test data (labels cannot cross
			// package boundaries)
			err := os.Rename(path + "/BUILD_", path + "/BUILD")
			g.Expect(err).To(BeNil())

			isPkg := pathutils.IsPackage(path)
			g.Expect(isPkg).To(BeTrue())
		})
	})

	t.Run("path is not a package", func(t *testing.T) {
		g := NewGomegaWithT(t)
		path := "testfixtures/workspace_1"
		isPkg := pathutils.IsPackage(path)
		g.Expect(isPkg).To(BeFalse())
	})
}

func TestFindWorkspaceRoot(t *testing.T) {

	t.Run("path is within a workspace", func(t *testing.T) {
		g := NewGomegaWithT(t)
		workspaceRoot, err := pathutils.FindWorkspaceRoot("testfixtures/workspace_1/pkg_1")
		g.Expect(err).To(BeNil())
		g.Expect(workspaceRoot).To(Equal("testfixtures/workspace_1"))
	})

	t.Run("path is not within a workspace", func(t *testing.T) {
		g := NewGomegaWithT(t)
		workspaceRoot, err := pathutils.FindWorkspaceRoot("testfixtures/")
		g.Expect(err).To(BeNil())
		g.Expect(workspaceRoot).To(Equal(""))
	})
}

func TestFindNearestParentPackage(t *testing.T) {

	t.Run("path is within a package", func(t *testing.T) {
		g := NewGomegaWithT(t)
		pkg, err := pathutils.FindNearestParentPackage("testfixtures/workspace_1/pkg_1/foo/bar")
		g.Expect(err).To(BeNil())
		g.Expect(pkg).
			To(Equal("testfixtures/workspace_1/pkg_1"))
	})

	t.Run("path is not within a package", func(t *testing.T) {
		g := NewGomegaWithT(t)
		pkg, err := pathutils.FindNearestParentPackage("testfixtures/")
		g.Expect(err).To(BeNil())
		g.Expect(pkg).To(Equal(""))
	})
}

func TestInvokeCmdInsideWorkspace(t *testing.T) {

	t.Run("invoke info inside a workspace", func(t *testing.T) {
		g := NewGomegaWithT(t)
		workingDirectory, err := os.Getwd()
		g.Expect(err).To(BeNil())
		// Set $HOME, which is needed to create a cache directory for Bazel
		err = os.Setenv("HOME", workingDirectory)
		g.Expect(err).To(BeNil())
		// cd into workspace within test fixtures
		err = os.Chdir(filepath.Join(workingDirectory, "testfixtures/workspace_1"))
		g.Expect(err).To(BeNil())

		err = pathutils.InvokeCmdInsideWorkspace("info", func() error {
			infoCmd := info.New(ioutils.Streams{})
			return infoCmd.Run(nil, nil)
		})

		g.Expect(err).To(BeNil())

		// cd back to original working directory
		err = os.Chdir(workingDirectory)
		g.Expect(err).To(BeNil())
	})

	t.Run("invoke info outside workspace", func(t *testing.T) {
		g := NewGomegaWithT(t)

		err := pathutils.InvokeCmdInsideWorkspace("info", func() error {
			infoCmd := info.New(ioutils.Streams{})
			return infoCmd.Run(nil, nil)
		})

		g.Expect(err).To(Equal(fmt.Errorf("the 'info' command is only supported from within a workspace " +
			"(below a directory having a WORKSPACE file)")))
	})
}
