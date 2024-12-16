/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package docs

import (
	"github.com/spf13/cobra"

	"github.com/aspect-build/aspect-cli/pkg/aspect/docs"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

func NewDefaultCmd() *cobra.Command {
	return NewCmd(ioutils.DefaultStreams)
}

func NewCmd(streams ioutils.Streams) *cobra.Command {
	v := docs.New(streams)

	cmd := &cobra.Command{
		Use:     "docs [topic]",
		Short:   "Open documentation in the browser",
		GroupID: "aspect",
		Long: `Given a selected topic, open the relevant API docs in a browser window.

The mechanism of choosing the browser to open is documented at https://github.com/pkg/browser
By default, opens bazel.build/docs`,
		Example: `# Open the Bazel glossary of terms
% aspect docs glossary

# Open the docs for the aspect-build/rules_js ruleset
% aspect docs aspect_rules_js`,
		RunE: v.Run,
	}

	return cmd
}
