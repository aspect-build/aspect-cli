/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package test

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

type Test struct {
	ioutils.Streams
	bzl bazel.Spawner
}

func New(streams ioutils.Streams, bzl bazel.Spawner) *Test {
	return &Test{
		Streams: streams,
		bzl:     bzl,
	}
}

func (testCmd *Test) Run(_ *cobra.Command, args []string) error {
	bazelCmd := []string{"test"}
	bazelCmd = append(bazelCmd, args...)

	if exitCode, err := testCmd.bzl.Spawn(bazelCmd); exitCode != 0 {
		err = &aspecterrors.ExitError{
			Err:      err,
			ExitCode: exitCode,
		}
		return err
	}

	return nil
}
