/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package pathutils

import (
	"testing"

	. "github.com/onsi/gomega"
	"github.com/spf13/cobra"
)

func getMockCmd(osGetwd func() (string, error)) *cobra.Command {
	return &cobra.Command{
		Use:   "mock",
		Short: "This is a mock command",
		Long:  "This is a mock command",
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
