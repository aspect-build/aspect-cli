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
	"errors"
	"fmt"
	"log"
	"net"
	"os"
	"time"

	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	"github.com/aspect-build/aspect-cli/gazelle/common/cache"
	"github.com/aspect-build/aspect-cli/gazelle/common/ibp"
	"github.com/aspect-build/aspect-cli/gazelle/common/watch"
	starzelleHost "github.com/aspect-build/aspect-cli/gazelle/languages/host"
	"github.com/aspect-build/aspect-cli/gazelle/runner"
	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/interceptors"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

func NewDefaultCmd() *cobra.Command {
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
				flags.FlagsInterceptor(ioutils.DefaultStreams),
			},
			func(ctx context.Context, cmd *cobra.Command, args []string) error {
				mode, _ := cmd.Flags().GetString("mode")
				exclude, _ := cmd.Flags().GetStringSlice("exclude")
				watch, _ := cmd.Flags().GetBool("watch")
				watchman, _ := cmd.Flags().GetBool("watchman")
				return run(ctx, mode, exclude, watch, watchman, args)
			},
		),
	}

	// TODO: restrict to only valid values (see https://github.com/spf13/pflag/issues/236)
	cmd.Flags().String("mode", "fix", "Method for emitting merged BUILD files.\n\tfix: write generated and merged files to disk\n\tprint: print files to stdout\n\tdiff: print a unified diff")
	cmd.Flags().StringSlice("exclude", []string{}, "Files to exclude from BUILD generation")
	cmd.Flags().Bool("watchman", false, "Use the EXPERIMENTAL watchman daemon to watch for changes across 'configure' invocations")
	cmd.Flags().Bool("watch", false, "Use the EXPERIMENTAL watch mode to watch for changes in the workspace and automatically 'configure' when files change")

	return cmd
}

func run(ctx context.Context, mode string, exclude []string, watch, watchman bool, args []string) error {
	if watch || watchman {
		cache.SetCacheFactory(cache.NewWatchmanCache)
	}

	v := runner.New()

	addCliEnabledLanguages(v)

	if len(v.Languages()) == 0 {
		fmt.Fprintln(ioutils.DefaultStreams.Stderr, `No languages enabled for BUILD file generation.

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

		return &aspecterrors.ExitError{
			ExitCode: aspecterrors.ConfigureNoConfig,
		}
	}

	if watch {
		// Watch mode has its own run/return/exit-code logic
		return runConfigureWatch(ctx, v, mode, exclude, args)
	}

	changed, err := v.Generate(mode, exclude, args)

	// Unique error codes for:
	// - internal errors
	// - files diffs
	// - files updated
	if err != nil {
		err = &aspecterrors.ExitError{
			ExitCode: aspecterrors.UnhandledOrInternalError,
			Err:      err,
		}
	} else if changed {
		if mode == "fix" {
			err = &aspecterrors.ExitError{
				ExitCode: aspecterrors.ConfigureFixed,
			}
		} else {
			err = &aspecterrors.ExitError{
				ExitCode: aspecterrors.ConfigureDiff,
			}
		}
	}

	return err
}

func runConfigureWatch(ctx context.Context, v *runner.GazelleRunner, mode string, exclude []string, args []string) error {
	abazel := ibp.NewServer()

	// Start listening for a connection immediately.
	if err := abazel.Serve(ctx); err != nil {
		return fmt.Errorf("failed to connect to aspect bazel protocol: %w", err)
	}

	// Close the watch protocol on complete, no matter what the status is
	defer abazel.Close()

	watchDone := make(chan struct{})

	// "Launch" the client in the background
	go func() {
		v.Watch(abazel.Address(), mode, exclude, args)
		close(watchDone)
	}()

	// Start the workspace watcher
	w := watch.NewWatchman(bazel.WorkspaceFromWd.WorkspaceRoot())
	if err := w.Start(); err != nil {
		return fmt.Errorf("failed to start the watcher: %w", err)
	}
	defer w.Close()

	// Since the Subscribe() method is blocking, we need to run a separate
	// goroutine to stop the watcher when we receive a signal to cancel the
	// process.
	go func() {
		select {
		case <-ctx.Done():
		case <-watchDone:
		}

		w.Close()
	}()

	// Wait for either the connection to be established or the timeout to occur.
	// Should be instant since the runner is in-process.
	select {
	case <-abazel.WaitForConnection():
	case <-time.After(10 * time.Second):
	}

	if !abazel.HasConnection() {
		return fmt.Errorf("no connection to incremental protocol")
	}

	for cs, err := range w.Subscribe(ctx, "aspect-configure-watch") {
		if err != nil {
			// Break the subscribe iteration if the context is done or if the watcher is closed.
			if errors.Is(err, context.Canceled) || errors.Is(err, net.ErrClosed) {
				break
			}

			return fmt.Errorf("failed to get next event: %w", err)
		}

		// Enter into the build state to discard supirious changes caused by Bazel reading the
		// inputs which leads to their atime to change.
		if err := w.StateEnter("aspect-configure-watch"); err != nil {
			return fmt.Errorf("failed to enter build state: %w", err)
		}

		if err := abazel.Cycle(changesetToCycle(cs)); err != nil {
			return fmt.Errorf("failed to send cycle to incremental protocol: %w", err)
		}

		// Leave the build state and fast forward the subscription clock.
		if err := w.StateLeave("aspect-configure-watch"); err != nil {
			return fmt.Errorf("failed to enter build state: %w", err)
		}
	}

	return nil
}

// Convert a watch.ChangeSet to ibp.SourceInfoMap
func changesetToCycle(cs *watch.ChangeSet) ibp.SourceInfoMap {
	b := true
	si := &ibp.SourceInfo{IsSource: &b}
	changes := make(ibp.SourceInfoMap, len(cs.Paths))
	for _, p := range cs.Paths {
		changes[p] = si
	}
	return changes
}

func addCliEnabledLanguages(c *runner.GazelleRunner) {
	// Order matters for gazelle languages. Proto should be run before golang.
	viper.SetDefault("configure.languages.protobuf", false)
	if viper.GetBool("configure.languages.protobuf") {
		c.AddLanguage(runner.Protobuf)
	}

	viper.SetDefault("configure.languages.go", false)
	if viper.GetBool("configure.languages.go") {
		if os.Getenv(runner.GO_REPOSITORY_CONFIG_ENV) == "" {
			goConfigPath, err := determineGoRepositoryConfigPath()
			if err != nil {
				log.Fatalf("ERROR: unable to determine go_repository config path: %v", err)
			}

			if goConfigPath != "" {
				os.Setenv(runner.GO_REPOSITORY_CONFIG_ENV, goConfigPath)
			}
		}
		c.AddLanguage(runner.Go)
	}

	viper.SetDefault("configure.languages.javascript", false)
	if viper.GetBool("configure.languages.javascript") {
		c.AddLanguage(runner.JavaScript)
	}

	viper.SetDefault("configure.languages.bzl", false)
	if viper.GetBool("configure.languages.bzl") {
		c.AddLanguage(runner.Bzl)
	}

	viper.SetDefault("configure.languages.python", false)
	if viper.GetBool("configure.languages.python") {
		c.AddLanguage(runner.Python)
	}

	viper.SetDefault("configure.languages.cc", false)
	if viper.GetBool("configure.languages.cc") {
		c.AddLanguage(runner.CC)
	}

	// Add additional starlark plugins
	if configurePlugins := viper.GetStringSlice("configure.plugins"); len(configurePlugins) > 0 {
		c.AddLanguageFactory(starzelleHost.GazelleLanguageName, func() language.Language {
			return starzelleHost.NewLanguage(configurePlugins...)
		})
	}
}
