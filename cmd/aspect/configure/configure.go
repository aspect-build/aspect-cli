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
	"context"
	"fmt"

	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	"github.com/aspect-build/aspect-cli/gazelle/common/cache"
	starzelleHost "github.com/aspect-build/aspect-cli/gazelle/host"
	"github.com/aspect-build/aspect-cli/pkg/aspect/configure"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	"github.com/aspect-build/aspect-cli/pkg/interceptors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

func NewDefaultCmd() *cobra.Command {
	return NewCmd(ioutils.DefaultStreams)
}

func NewCmd(streams ioutils.Streams) *cobra.Command {
	return NewCmdWithConfigure(streams, configure.New(streams))
}
func NewCmdWithConfigure(streams ioutils.Streams, v configure.ConfigureRunner) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "configure",
		Short: "Auto-configure Bazel by updating BUILD files",
		Long: `configure generates and updates BUILD files from source code.

It is named after the "make configure" workflow which is typical in C++ projects, using
[autoconf](https://www.gnu.org/software/autoconf/).

configure is non-destructive: hand-edits to BUILD files are generally preserved.
You can use a ` + "`# keep`" + ` directive to force the tool to leave existing BUILD contents alone.
Run 'aspect help directives' for more documentation on directives.

So far these languages are supported:
- Go and Protocol Buffers, thanks to code from [gazelle]
- Python, thanks to code from [rules_python]
- JavaScript (including TypeScript)
- Kotlin (experimental, see https://github.com/aspect-build/aspect-cli/issues/474)
- Starlark, thanks to code from [bazel-skylib]

configure is based on [gazelle]. We are very grateful to the authors of that software.
The advantage of configure in Aspect CLI is that you don't need to compile the tooling before running it.

[gazelle]: https://github.com/bazelbuild/bazel-gazelle
[rules_python]: https://github.com/bazelbuild/rules_python/tree/main/gazelle
[bazel-skylib]: https://github.com/bazelbuild/bazel-skylib/tree/main/gazelle

To change the behavior of configure, you add "directives" to your BUILD files, which are comments
in a special syntax.
Run 'aspect help directives' or see https://docs.aspect.build/cli/help/directives for more info.
`,
		GroupID: "aspect",
		RunE: interceptors.Run(
			[]interceptors.Interceptor{
				flags.FlagsInterceptor(streams),
			},
			func(_ context.Context, cmd *cobra.Command, args []string) error {
				mode, _ := cmd.Flags().GetString("mode")
				exclude, _ := cmd.Flags().GetStringSlice("exclude")
				watch, _ := cmd.Flags().GetBool("watch")
				watchman, _ := cmd.Flags().GetBool("watchman")
				return run(streams, v, mode, exclude, watch, watchman, args)
			},
		),
	}

	// TODO: restrict to only valid values (see https://github.com/spf13/pflag/issues/236)
	cmd.Flags().String("mode", "fix", "Method for emitting merged BUILD files.\n\tfix: write generated and merged files to disk\n\tprint: print files to stdout\n\tdiff: print a unified diff")
	cmd.Flags().StringSlice("exclude", []string{}, "Files to exclude from BUILD generation")
	cmd.Flags().Bool("watchman", false, "Use the EXPERIMENTAL watchman daemon to watch for changes across 'configure' invocations")
	cmd.Flags().Bool("watch", false, "Use the EXPERIMENTAL watch mode to watch for changes in the workspace and automatically 'configure' when files change")

	addCliEnabledLanguages(v)

	return cmd
}

func run(streams ioutils.Streams, v configure.ConfigureRunner, mode string, exclude []string, watch, watchman bool, args []string) error {
	if watch || watchman {
		cache.SetCacheFactory(cache.NewWatchmanCache)
	}

	var err error
	if watch {
		err = v.Watch(mode, exclude, args)
	} else {
		err = v.Generate(mode, exclude, args)
	}

	if aspectError, isAError := err.(*aspecterrors.ExitError); isAError && aspectError.ExitCode == aspecterrors.ConfigureNoConfig {
		fmt.Fprintln(streams.Stderr, `No languages enabled for BUILD file generation.

To enable one or more languages, add the following to the .aspect/cli/config.yaml
file in your WORKSPACE or home directory and enable/disable languages as needed:

configure:
  languages:
	javascript: true
	go: true
	protobuf: true
	bzl: true
	python: true
  plugins:
    path/to/starlark/plugin.star`)
	}
	return err
}

func addCliEnabledLanguages(c configure.ConfigureRunner) {
	// Order matters for gazelle languages. Proto should be run before golang.
	viper.SetDefault("configure.languages.protobuf", false)
	if viper.GetBool("configure.languages.protobuf") {
		c.AddLanguage(configure.Protobuf)
	}

	viper.SetDefault("configure.languages.go", false)
	if viper.GetBool("configure.languages.go") {
		c.AddLanguage(configure.Go)
	}

	viper.SetDefault("configure.languages.javascript", false)
	if viper.GetBool("configure.languages.javascript") {
		c.AddLanguage(configure.JavaScript)
	}

	viper.SetDefault("configure.languages.bzl", false)
	if viper.GetBool("configure.languages.bzl") {
		c.AddLanguage(configure.Bzl)
	}

	viper.SetDefault("configure.languages.python", false)
	if viper.GetBool("configure.languages.python") {
		c.AddLanguage(configure.Python)
	}

	viper.SetDefault("configure.languages.cc", false)
	if viper.GetBool("configure.languages.cc") {
		c.AddLanguage(configure.CC)
	}

	// Add additional starlark plugins
	if configurePlugins := viper.GetStringSlice("configure.plugins"); len(configurePlugins) > 0 {
		c.AddLanguageFactory(starzelleHost.GazelleLanguageName, func() language.Language {
			return starzelleHost.NewLanguage(configurePlugins...)
		})
	}
}
