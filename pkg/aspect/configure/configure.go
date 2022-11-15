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

	"aspect.build/cli/pkg/aspect/root/config"
	"aspect.build/cli/pkg/ioutils"
	"github.com/bazelbuild/bazel-gazelle/language"
	golang "github.com/bazelbuild/bazel-gazelle/language/go"
	"github.com/bazelbuild/bazel-gazelle/language/proto"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"
)

type Configure struct {
	ioutils.Streams

	additionalLanguages map[string]language.Language
}

func New(streams ioutils.Streams, additionalLanguages map[string]language.Language) *Configure {
	return &Configure{
		Streams:             streams,
		additionalLanguages: additionalLanguages,
	}
}

func pluralize(s string, num int) string {
	if num == 1 {
		return s
	} else {
		return s + "s"
	}
}

func (v *Configure) Run(_ context.Context, _ *cobra.Command, args []string) error {
	languages := make([]language.Language, 0, 32)
	languageKeys := make([]string, 0, 32)

	// Order matters for gazelle languages. Proto should be run before golang.
	viper.SetDefault("configure.languages.protobuf", true)
	if viper.GetBool("configure.languages.protobuf") {
		languages = append(languages, proto.NewLanguage())
		languageKeys = append(languageKeys, "protobuf")
	}

	viper.SetDefault("configure.languages.go", true)
	if viper.GetBool("configure.languages.go") {
		languages = append(languages, golang.NewLanguage())
		languageKeys = append(languageKeys, "go")
	}

	for key, language := range v.additionalLanguages {
		languages = append(languages, language)
		languageKeys = append(languageKeys, key)
	}

	if len(languageKeys) != 0 {
		fmt.Fprintf(v.Streams.Stdout, "Updating BUILD files for %s\n", strings.Join(languageKeys, ", "))
	}

	viper.SetDefault("configure.languages.javascript", true)
	if viper.GetBool("configure.languages.javascript") && v.additionalLanguages["javascript"] == nil {
		// Let the user know that this language is available in Pro
		workspaceConfigFile, err := config.WorkspaceConfigFile()
		if err != nil {
			return err
		}
		fmt.Fprintf(v.Streams.Stderr, `
===============================================================================
JavaScript and TypeScript BUILD file generation is available in Aspect CLI Pro.
Run 'aspect pro' to enable Pro features -or- to turn off this message add the
following lines to %s:

configure:
  languages:
    javascript: false
===============================================================================

`, workspaceConfigFile)
	}

	if len(languageKeys) == 0 {
		fmt.Fprintln(v.Streams.Stderr, "No languages configured for BUILD file generation")
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

	fmt.Fprintf(v.Streams.Stdout, "%v BUILD %s visited\n", stats.NumBuildFilesVisited, pluralize("file", stats.NumBuildFilesVisited))
	fmt.Fprintf(v.Streams.Stdout, "%v BUILD %s updated\n", stats.NumBuildFilesUpdated, pluralize("file", stats.NumBuildFilesUpdated))
	return nil
}
