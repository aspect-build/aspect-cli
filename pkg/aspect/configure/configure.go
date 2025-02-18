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
	"log"
	"os"
	"strings"

	bzl "github.com/aspect-build/aspect-cli/gazelle/bzl"
	"github.com/aspect-build/aspect-cli/gazelle/common/progress"
	js "github.com/aspect-build/aspect-cli/gazelle/js"
	kotlin "github.com/aspect-build/aspect-cli/gazelle/kotlin"
	python "github.com/aspect-build/aspect-cli/gazelle/python"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	"github.com/aspect-build/aspect-cli/pkg/hints"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/bazelbuild/bazel-gazelle/language"
	golang "github.com/bazelbuild/bazel-gazelle/language/go"
	"github.com/bazelbuild/bazel-gazelle/language/proto"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"
	"golang.org/x/term"
)

type Configure struct {
	ioutils.Streams

	languageKeys []string
	languages    []func() language.Language
}

func New(streams ioutils.Streams) *Configure {
	c := &Configure{
		Streams: streams,
	}

	c.addDefaultLanguages()

	if os.Getenv("CONFIGURE_PROGRESS") != "" && term.IsTerminal(int(os.Stdout.Fd())) {
		c.AddLanguage("progress", progress.NewLanguage)
	}

	return c
}

func pluralize(s string, num int) string {
	if num == 1 {
		return s
	} else {
		return s + "s"
	}
}

func (c *Configure) AddLanguage(lang string, langFactory func() language.Language) {
	c.languageKeys = append(c.languageKeys, lang)
	c.languages = append(c.languages, langFactory)
}

func (c *Configure) addDefaultLanguages() {
	// Order matters for gazelle languages. Proto should be run before golang.
	viper.SetDefault("configure.languages.protobuf", false)
	if viper.GetBool("configure.languages.protobuf") {
		c.AddLanguage("protobuf", proto.NewLanguage)
	}

	viper.SetDefault("configure.languages.go", false)
	if viper.GetBool("configure.languages.go") {
		if os.Getenv(GO_REPOSITORY_CONFIG_ENV) == "" {
			goConfigPath, err := determineGoRepositoryConfigPath()
			if err != nil {
				log.Fatalf("ERROR: unable to determine go_repository config path: %v", err)
			}

			if goConfigPath != "" {
				os.Setenv(GO_REPOSITORY_CONFIG_ENV, goConfigPath)
			}
		}

		c.AddLanguage("go", golang.NewLanguage)
	}

	viper.SetDefault("configure.languages.javascript", false)
	if viper.GetBool("configure.languages.javascript") {
		c.AddLanguage("javascript", js.NewLanguage)
	}

	viper.SetDefault("configure.languages.kotlin", false)
	if viper.GetBool("configure.languages.kotlin") {
		c.AddLanguage("kotlin", kotlin.NewLanguage)
	}

	viper.SetDefault("configure.languages.bzl", false)
	if viper.GetBool("configure.languages.bzl") {
		c.AddLanguage("bzl", bzl.NewLanguage)
	}

	viper.SetDefault("configure.languages.python", false)
	if viper.GetBool("configure.languages.python") {
		c.AddLanguage("python", python.NewLanguage)
	}
}

func (runner *Configure) Run(_ context.Context, cmd *cobra.Command, args []string) error {
	if len(runner.languageKeys) == 0 {
		fmt.Fprintln(runner.Streams.Stderr, `No languages enabled for BUILD file generation.

To enable one or more languages, add the following to the .aspect/cli/config.yaml
file in your WORKSPACE or home directory and enable/disable languages as needed:

configure:
  languages:
    javascript: true
    go: true
    kotlin: true
    protobuf: true`)
		return &aspecterrors.ExitError{
			ExitCode: aspecterrors.ConfigureNoConfig,
		}
	}

	var err error
	var wd string
	if wd, err = os.Getwd(); err != nil {
		log.Fatal(err)
	}

	// Append the aspect-cli mode flag to the args parsed by gazelle.
	mode, _ := cmd.Flags().GetString("mode")

	fixArgs := []string{"--mode=" + mode}

	// gazelle --cpuprofile enabled via environment variable.
	cpuprofile := os.Getenv("GAZELLE_CPUPROFILE")
	if cpuprofile != "" {
		fixArgs = append(fixArgs, "--cpuprofile="+cpuprofile)
	}

	// gazelle --memprofile enabled via environment variable.
	memprofile := os.Getenv("GAZELLE_MEMPROFILE")
	if memprofile != "" {
		fixArgs = append(fixArgs, "--memprofile="+memprofile)
	}

	go_repo_config := os.Getenv(GO_REPOSITORY_CONFIG_ENV)
	if go_repo_config != "" {
		fixArgs = append(fixArgs, "--repo_config="+go_repo_config)
	}

	// Append additional args including specific directories to fix.
	fixArgs = append(fixArgs, args...)

	if mode == "fix" {
		fmt.Fprintf(runner.Streams.Stdout, "Updating BUILD files for %s\n", strings.Join(runner.languageKeys, ", "))
	}

	// Instantiate all the languages
	languages := make([]language.Language, 0, len(runner.languages))
	for _, lang := range runner.languages {
		languages = append(languages, lang())
	}

	// swap os.Stdout, os.Stderr and ioutils.DefaultStreams for hints
	// before calling runFixUpdate. We can't easily control where gazelle plugins
	// write to so we swap these instead to capture all of their outputs.
	oldStdout := os.Stdout
	oldStderr := os.Stderr
	oldDefaultStreams := ioutils.DefaultStreams
	os.Stdout = hints.Stdout
	os.Stderr = hints.Stderr
	log.Default().SetOutput(hints.Stderr)

	stats, err := runFixUpdate(wd, languages, updateCmd, fixArgs)

	// Swap back
	log.Default().SetOutput(oldStderr)
	os.Stdout = oldStdout
	os.Stderr = oldStderr
	ioutils.DefaultStreams = oldDefaultStreams

	exitCode := aspecterrors.OK

	// Unique error codes for changes fixed vs diffs, otherwise fallback to bazel unhandled error code.
	if err == errExit {
		exitCode = aspecterrors.ConfigureDiff
		err = nil
	} else if err == resultFileChanged {
		exitCode = aspecterrors.ConfigureFixed
		err = nil
	} else if err != nil {
		exitCode = aspecterrors.UnhandledOrInternalError
	}

	if mode == "fix" && stats != nil {
		fmt.Fprintf(runner.Streams.Stdout, "%v BUILD %s visited\n", stats.NumBuildFilesVisited, pluralize("file", stats.NumBuildFilesVisited))
		fmt.Fprintf(runner.Streams.Stdout, "%v BUILD %s updated\n", stats.NumBuildFilesUpdated, pluralize("file", stats.NumBuildFilesUpdated))
	}

	return &aspecterrors.ExitError{
		ExitCode: exitCode,
		Err:      err,
	}
}
