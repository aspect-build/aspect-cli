/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package docs

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/docs"
	"aspect.build/cli/pkg/ioutils"
)

func NewDefaultDocsCmd() *cobra.Command {
	return NewDocsCmd(ioutils.DefaultStreams)
}

func NewDocsCmd(streams ioutils.Streams) *cobra.Command {
	v := docs.New(streams)

	cmd := &cobra.Command{
		Use:   "docs",
		Short: "Open documentation in the browser.",
		Long: `Given a selected topic, open the relevant API docs in a browser window.
The mechanism of choosing the browser to open is documented at https://github.com/pkg/browser
By default, opens bazel.build/docs`,
		RunE: v.Run,
	}

	return cmd
}
