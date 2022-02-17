/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package version

import (
	"context"

	"github.com/spf13/cobra"

	"aspect.build/cli/buildinfo"
	"aspect.build/cli/pkg/aspect/version"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultVersionCmd() *cobra.Command {
	return NewVersionCmd(ioutils.DefaultStreams, bazel.New())
}

func NewVersionCmd(streams ioutils.Streams, bzl bazel.Bazel) *cobra.Command {
	v := version.New(streams)

	v.BuildinfoRelease = buildinfo.Release
	v.BuildinfoGitStatus = buildinfo.GitStatus

	cmd := &cobra.Command{
		Use:   "version",
		Short: "Print the version of aspect CLI as well as tools it invokes.",
		Long:  `Prints version info on colon-separated lines, just like bazel does`,
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				interceptors.WorkspaceRootInterceptor(),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) (exitErr error) {
				workspaceRoot := ctx.Value(interceptors.WorkspaceRootKey).(string)
				bzl.SetWorkspaceRoot(workspaceRoot)
				return v.Run(bzl)
			},
		),
	}

	cmd.PersistentFlags().BoolVarP(&v.GNUFormat, "gnu_format", "", false, "format space-separated following GNU convention")

	return cmd
}
