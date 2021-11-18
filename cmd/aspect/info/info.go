/*
Copyright © 2021 Aspect Build Systems

Not licensed for re-use
*/

package info

import (
	"aspect.build/cli/pkg/pathutils"
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/info"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultInfoCmd() *cobra.Command {
	return NewInfoCmd(ioutils.DefaultStreams)
}

func NewInfoCmd(streams ioutils.Streams) *cobra.Command {
	i := info.New(streams)

	cmd := &cobra.Command{
		Use:   "info",
		Short: "Displays runtime info about the bazel server.",
		Long: `Displays information about the state of the bazel process in the
form of several "key: value" pairs.  This includes the locations of
several output directories.  Because some of the
values are affected by the options passed to 'bazel build', the
info command accepts the same set of options.

A single non-option argument may be specified (e.g. "bazel-bin"), in
which case only the value for that key will be printed.

If --show_make_env is specified, the output includes the set of key/value
pairs in the "Make" environment, accessible within BUILD files.

The full list of keys and the meaning of their values is documented in
the bazel User Manual, and can be programmatically obtained with
'bazel help info-keys'.

See also 'bazel version' for more detailed bazel version
information.`,
		Args: cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) (exitErr error) {
			return pathutils.InvokeCmdInsideWorkspace("info", func() error {
				return i.Run(cmd, args)
			})
		},
	}

	cmd.PersistentFlags().BoolVarP(&i.ShowMakeEnv, "show_make_env", "", false, `include the set of key/value pairs in the "Make" environment,
accessible within BUILD files`)
	return cmd
}
