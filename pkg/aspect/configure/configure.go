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

	bzl "aspect.build/cli/gazelle/bzl"
	js "aspect.build/cli/gazelle/js"
	kotlin "aspect.build/cli/gazelle/kotlin"
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/ioutils"
	"github.com/bazelbuild/bazel-gazelle/language"
	golang "github.com/bazelbuild/bazel-gazelle/language/go"
	"github.com/bazelbuild/bazel-gazelle/language/proto"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"
)

type Configure struct {
	ioutils.Streams

	languageKeys []string
	languages    []language.Language
}

func New(streams ioutils.Streams) *Configure {
	c := &Configure{
		Streams: streams,
	}

	c.addDefaultLanguages()

	return c
}

func pluralize(s string, num int) string {
	if num == 1 {
		return s
	} else {
		return s + "s"
	}
}

func (c *Configure) AddLanguage(lang string, l language.Language) {
	c.languageKeys = append(c.languageKeys, lang)
	c.languages = append(c.languages, l)
}

func (c *Configure) addDefaultLanguages() {
	// Order matters for gazelle languages. Proto should be run before golang.
	viper.SetDefault("configure.languages.protobuf", false)
	if viper.GetBool("configure.languages.protobuf") {
		c.AddLanguage("protobuf", proto.NewLanguage())
	}

	viper.SetDefault("configure.languages.go", false)
	if viper.GetBool("configure.languages.go") {
		c.AddLanguage("go", golang.NewLanguage())
	}

	viper.SetDefault("configure.languages.javascript", false)
	if viper.GetBool("configure.languages.javascript") {
		c.AddLanguage("javascript", js.NewLanguage())
	}

	viper.SetDefault("configure.languages.kotlin", false)
	if viper.GetBool("configure.languages.kotlin") {
		c.AddLanguage("kotlin", kotlin.NewLanguage())
	}

	viper.SetDefault("configure.languages.bzl", false)
	if viper.GetBool("configure.languages.bzl") {
		c.AddLanguage("bzl", bzl.NewLanguage())
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

	if mode == "fix" {
		fmt.Fprintf(runner.Streams.Stdout, "Updating BUILD files for %s\n", strings.Join(runner.languageKeys, ", "))
	}

	stats, err := runFixUpdate(wd, runner.languages, updateCmd, []string{"--mode=" + mode})

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

	if mode == "fix" {
		fmt.Fprintf(runner.Streams.Stdout, "%v BUILD %s visited\n", stats.NumBuildFilesVisited, pluralize("file", stats.NumBuildFilesVisited))
		fmt.Fprintf(runner.Streams.Stdout, "%v BUILD %s updated\n", stats.NumBuildFilesUpdated, pluralize("file", stats.NumBuildFilesUpdated))
	}

	return &aspecterrors.ExitError{
		ExitCode: exitCode,
		Err:      err,
	}
}
