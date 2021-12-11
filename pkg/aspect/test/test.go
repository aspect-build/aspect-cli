/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package test

import (
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspecterrors"
)

type Test struct {
	ioutils.Streams
	bzl bazel.Bazel
}

func New(streams ioutils.Streams, bzl bazel.Bazel) *Test {
	return &Test{
		Streams: streams,
		bzl:     bzl,
	}
}

func (t *Test) Run(_ *cobra.Command, args []string) error {
	bazelCmd := []string{"test"}
	bazelCmd = append(bazelCmd, args...)

	if exitCode, err := t.bzl.Spawn(bazelCmd); exitCode != 0 {
		err = &aspecterrors.ExitError{
			Err:      err,
			ExitCode: exitCode,
		}
		return err
	}

	return nil
}
