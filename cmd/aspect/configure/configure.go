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

package configure

import (
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/configure"
	"aspect.build/cli/pkg/aspect/root/flags"
	"aspect.build/cli/pkg/interceptors"
	"aspect.build/cli/pkg/ioutils"
	"github.com/bazelbuild/bazel-gazelle/language"
	golang "github.com/bazelbuild/bazel-gazelle/language/go"
	"github.com/bazelbuild/bazel-gazelle/language/proto"
)

func NewDefaultConfigureCmd() *cobra.Command {
	var languages = []language.Language{
		proto.NewLanguage(),
		golang.NewLanguage(),
	}

	return NewConfigureCmd(
		ioutils.DefaultStreams,
		languages,
		"Generate and update BUILD files for Golang and Protobuf",
		"Generates and updates BUILD files from sources for Golang and Protobuf.",
	)
}

func NewConfigureCmd(streams ioutils.Streams, languages []language.Language, shortDesc string, longDesc string) *cobra.Command {
	v := configure.New(streams, languages)

	cmd := &cobra.Command{
		Use:     "configure",
		Short:   shortDesc,
		Long:    longDesc,
		GroupID: "aspect",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			v.Run,
		),
	}
	return cmd
}
