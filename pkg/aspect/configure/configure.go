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
	"fmt"
	"log"
	"os"
	"strings"

	cc "github.com/EngFlow/gazelle_cc/language/cc"
	bzl "github.com/aspect-build/aspect-cli/gazelle/bzl"
	"github.com/aspect-build/aspect-cli/gazelle/common/progress"
	js "github.com/aspect-build/aspect-cli/gazelle/js"
	kotlin "github.com/aspect-build/aspect-cli/gazelle/kotlin"
	python "github.com/aspect-build/aspect-cli/gazelle/python"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/bazelbuild/bazel-gazelle/language"
	golang "github.com/bazelbuild/bazel-gazelle/language/go"
	"github.com/bazelbuild/bazel-gazelle/language/proto"
	"golang.org/x/term"
)

type ConfigureRunner interface {
	AddLanguage(lang ConfigureLanguage)
	AddLanguageFactory(lang string, langFactory func() language.Language)
	Generate(mode ConfigureMode, excludes []string, args []string) error
}

type Configure struct {
	ioutils.Streams

	languageKeys []string
	languages    []func() language.Language
}

var _ ConfigureRunner = (*Configure)(nil)

// Builtin Gazelle languages
type ConfigureLanguage = string

const (
	JavaScript ConfigureLanguage = "javascript"
	Go                           = "go"
	Kotlin                       = "kotlin"
	Protobuf                     = "protobuf"
	Bzl                          = "bzl"
	Python                       = "python"
	CC                           = "cc"
)

// Gazelle --mode
type ConfigureMode = string

const (
	Fix   ConfigureMode = "fix"
	Print               = "update"
	Diff                = "diff"
)

func New(streams ioutils.Streams) *Configure {
	c := &Configure{
		Streams: streams,
	}

	if os.Getenv("CONFIGURE_PROGRESS") != "" && term.IsTerminal(int(os.Stdout.Fd())) {
		c.AddLanguageFactory("progress", progress.NewLanguage)
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

func (c *Configure) AddLanguageFactory(lang string, langFactory func() language.Language) {
	c.languageKeys = append(c.languageKeys, lang)
	c.languages = append(c.languages, langFactory)
}

func (c *Configure) AddLanguage(lang ConfigureLanguage) {
	switch lang {
	case JavaScript:
		c.AddLanguageFactory(lang, js.NewLanguage)
	case Go:
		if os.Getenv(GO_REPOSITORY_CONFIG_ENV) == "" {
			goConfigPath, err := determineGoRepositoryConfigPath()
			if err != nil {
				log.Fatalf("ERROR: unable to determine go_repository config path: %v", err)
			}

			if goConfigPath != "" {
				os.Setenv(GO_REPOSITORY_CONFIG_ENV, goConfigPath)
			}
		}
		c.AddLanguageFactory(lang, golang.NewLanguage)
	case Kotlin:
		c.AddLanguageFactory(lang, kotlin.NewLanguage)
	case Protobuf:
		c.AddLanguageFactory(lang, proto.NewLanguage)
	case Bzl:
		c.AddLanguageFactory(lang, bzl.NewLanguage)
	case Python:
		c.AddLanguageFactory(lang, python.NewLanguage)
	case CC:
		c.AddLanguageFactory(lang, cc.NewLanguage)
	default:
		log.Fatalf("ERROR: unknown language %q", lang)
	}
}

func (runner *Configure) Generate(mode ConfigureMode, excludes []string, args []string) error {
	if len(runner.languageKeys) == 0 {
		return &aspecterrors.ExitError{
			ExitCode: aspecterrors.ConfigureNoConfig,
		}
	}

	var wd string
	if wsRoot := bazel.WorkspaceFromWd.WorkspaceRoot(); wsRoot != "" {
		wd = wsRoot
	} else {
		var err error
		if wd, err = os.Getwd(); err != nil {
			log.Fatal(err)
		}
	}

	// Append the aspect-cli mode flag to the args parsed by gazelle.
	fixArgs := []string{"--mode=" + mode}

	for _, exclude := range excludes {
		fixArgs = append(fixArgs, "--exclude="+exclude)
	}

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

	stats, err := RunGazelleFixUpdate(wd, languages, fixArgs)

	if mode == "fix" && stats != nil {
		fmt.Fprintf(runner.Streams.Stdout, "%v BUILD %s visited\n", stats.NumBuildFilesVisited, pluralize("file", stats.NumBuildFilesVisited))
		fmt.Fprintf(runner.Streams.Stdout, "%v BUILD %s updated\n", stats.NumBuildFilesUpdated, pluralize("file", stats.NumBuildFilesUpdated))
	}

	var exitCode int

	// Unique error codes for:
	// - files diffs
	// - files updated
	// - internal errors
	if err == errExit {
		exitCode = aspecterrors.ConfigureDiff
		err = nil
	} else if err != nil {
		exitCode = aspecterrors.UnhandledOrInternalError
	} else if stats != nil && stats.NumBuildFilesUpdated > 0 {
		exitCode = aspecterrors.ConfigureFixed
	} else {
		return nil
	}

	return &aspecterrors.ExitError{
		ExitCode: exitCode,
		Err:      err,
	}
}
