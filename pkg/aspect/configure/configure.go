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

	js "aspect.build/cli/gazelle/js"
	kotlin "aspect.build/cli/gazelle/kotlin"
	"aspect.build/cli/pkg/ioutils"
	"github.com/bazelbuild/bazel-gazelle/language"
	golang "github.com/bazelbuild/bazel-gazelle/language/go"
	"github.com/bazelbuild/bazel-gazelle/language/proto"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"
)

type Configure struct {
	ioutils.Streams
}

func New(streams ioutils.Streams) *Configure {
	return &Configure{
		Streams: streams,
	}
}

func pluralize(s string, num int) string {
	if num == 1 {
		return s
	} else {
		return s + "s"
	}
}

func (runner *Configure) Run(_ context.Context, _ *cobra.Command, args []string) error {
	languages := make([]language.Language, 0, 32)
	languageKeys := make([]string, 0, 32)

	// Order matters for gazelle languages. Proto should be run before golang.
	viper.SetDefault("configure.languages.protobuf", false)
	if viper.GetBool("configure.languages.protobuf") {
		languages = append(languages, proto.NewLanguage())
		languageKeys = append(languageKeys, "protobuf")
	}

	viper.SetDefault("configure.languages.go", false)
	if viper.GetBool("configure.languages.go") {
		languages = append(languages, golang.NewLanguage())
		languageKeys = append(languageKeys, "go")
	}

	viper.SetDefault("configure.languages.javascript", false)
	if viper.GetBool("configure.languages.javascript") {
		languages = append(languages, js.NewLanguage())
		languageKeys = append(languageKeys, "javascript")
	}

	viper.SetDefault("configure.languages.kotlin", false)
	if viper.GetBool("configure.languages.kotlin") {
		languages = append(languages, kotlin.NewLanguage())
		languageKeys = append(languageKeys, "kotlin")
	}

	if len(languageKeys) != 0 {
		fmt.Fprintf(runner.Streams.Stdout, "Updating BUILD files for %s\n", strings.Join(languageKeys, ", "))
	}

	if len(languageKeys) == 0 {
		fmt.Fprintln(runner.Streams.Stderr, `No languages enabled for BUILD file generation.

To enable one or more languages, add the following to the .aspect/cli/config.yaml
file in your WORKSPACE or home directory and enable/disable languages as needed:

configure:
  languages:
    javascript: true
    go: true
    kotlin: true
    protobuf: true`)
		return nil
	}

	var err error
	var wd string
	if wd, err = os.Getwd(); err != nil {
		log.Fatal(err)
	}

	stats, err := runFixUpdate(wd, languages, updateCmd, args)
	if err != nil {
		return err
	}

	fmt.Fprintf(runner.Streams.Stdout, "%v BUILD %s visited\n", stats.NumBuildFilesVisited, pluralize("file", stats.NumBuildFilesVisited))
	fmt.Fprintf(runner.Streams.Stdout, "%v BUILD %s updated\n", stats.NumBuildFilesUpdated, pluralize("file", stats.NumBuildFilesUpdated))
	return nil
}
