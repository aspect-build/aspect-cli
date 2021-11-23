package pathutils

import (
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/spf13/cobra"
	. "github.com/onsi/gomega"

	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/pathutils"
	infocmd "aspect.build/cli/cmd/aspect/info"
	infopkg "aspect.build/cli/pkg/aspect/info"
)

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

		infoCmd := infopkg.New(ioutils.Streams{})
		err = infoCmd.Run(nil, nil)

		err = pathutils.InvokeCmdInsideWorkspace(func(cmd *cobra.Command, args []string) error {
			infoCmd := infopkg.New(ioutils.Streams{})
			return infoCmd.Run(nil, nil)
		})(infocmd.NewDefaultInfoCmd(), []string{})

		g.Expect(err).To(BeNil())

		// cd back to original working directory
		err = os.Chdir(workingDirectory)
		g.Expect(err).To(BeNil())
	})

	t.Run("invoke info outside workspace", func(t *testing.T) {
		g := NewGomegaWithT(t)

		err := pathutils.InvokeCmdInsideWorkspace(func(cmd *cobra.Command, args []string) error {
			infoCmd := infopkg.New(ioutils.Streams{})
			return infoCmd.Run(nil, nil)
		})(infocmd.NewDefaultInfoCmd(), []string{})

		g.Expect(err).To(Equal(fmt.Errorf("the 'info' command is only supported from within a workspace " +
			"(below a directory having a WORKSPACE file)")))
	})
}