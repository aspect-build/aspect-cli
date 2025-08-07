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
	"path"
	"strings"

	cc "github.com/EngFlow/gazelle_cc/language/cc"
	bzl "github.com/aspect-build/aspect-cli/gazelle/bzl"
	"github.com/aspect-build/aspect-cli/gazelle/common/git"
	"github.com/aspect-build/aspect-cli/gazelle/common/progress"
	js "github.com/aspect-build/aspect-cli/gazelle/js"
	python "github.com/aspect-build/aspect-cli/gazelle/python"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	watcher "github.com/aspect-build/aspect-cli/pkg/watch"
	"github.com/bazelbuild/bazel-gazelle/language"
	golang "github.com/bazelbuild/bazel-gazelle/language/go"
	"github.com/bazelbuild/bazel-gazelle/language/proto"
	"go.opentelemetry.io/otel"
	traceAttr "go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/trace"
	"golang.org/x/term"
)

type ConfigureRunner interface {
	AddLanguage(lang ConfigureLanguage)
	AddLanguageFactory(lang string, langFactory func() language.Language)
	Generate(mode ConfigureMode, excludes []string, args []string) error
	Watch(ctx context.Context, mode ConfigureMode, excludes []string, args []string) error
}

type Configure struct {
	ioutils.Streams

	tracer trace.Tracer

	languageKeys []string
	languages    []func() language.Language
}

var _ ConfigureRunner = (*Configure)(nil)

// Builtin Gazelle languages
type ConfigureLanguage = string

const (
	JavaScript ConfigureLanguage = "javascript"
	Go                           = "go"
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

// Setup the 'configure' support for gitignore within Gazelle.
func init() {
	git.SetupGitIgnore()
}

func New(streams ioutils.Streams) *Configure {
	c := &Configure{
		Streams: streams,
		tracer:  otel.GetTracerProvider().Tracer("aspect-configure"),
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

func (runner *Configure) PrepareGazelleArgs(mode ConfigureMode, excludes []string, args []string) (string, []string) {
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

	return wd, fixArgs
}

// Instantiate an instance of each language enabled in this Configure instance.
func (runner *Configure) InstantiateLanguages() []language.Language {
	languages := make([]language.Language, 0, len(runner.languages))
	for _, lang := range runner.languages {
		languages = append(languages, lang())
	}
	return languages
}

func (runner *Configure) Generate(mode ConfigureMode, excludes []string, args []string) error {
	if len(runner.languageKeys) == 0 {
		return &aspecterrors.ExitError{
			ExitCode: aspecterrors.ConfigureNoConfig,
		}
	}

	_, t := runner.tracer.Start(context.Background(), "Configure.Generate", trace.WithAttributes(
		traceAttr.String("mode", mode),
		traceAttr.StringSlice("languages", runner.languageKeys),
		traceAttr.StringSlice("excludes", excludes),
		traceAttr.StringSlice("args", args),
	))
	defer t.End()

	wd, fixArgs := runner.PrepareGazelleArgs(mode, excludes, args)

	if mode == "fix" {
		fmt.Fprintf(runner.Streams.Stdout, "Updating BUILD files for %s\n", strings.Join(runner.languageKeys, ", "))
	}

	// Run gazelle
	stats, err := RunGazelleFixUpdate(wd, runner.InstantiateLanguages(), fixArgs)

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
func (p *Configure) Watch(ctx context.Context, mode ConfigureMode, excludes []string, args []string) error {
	// Params for the underlying gazelle call
	wd, fixArgs := p.PrepareGazelleArgs(mode, excludes, args)

	// Initial run and status update to stdout.
	fmt.Fprintf(p.Streams.Stdout, "Initialize BUILD file generation --watch in %v\n", wd)
	languages := p.InstantiateLanguages()
	stats, err := RunGazelleFixUpdate(wd, languages, fixArgs)
	if err != nil {
		return fmt.Errorf("failed to run gazelle fix/update: %w", err)
	}
	if stats.NumBuildFilesUpdated > 0 {
		fmt.Fprintf(p.Streams.Stdout, "Initial %v/%v BUILD files updated\n", stats.NumBuildFilesUpdated, stats.NumBuildFilesVisited)
	} else {
		fmt.Fprintf(p.Streams.Stdout, "Initial %v BUILD files visited\n", stats.NumBuildFilesVisited)
	}

	// Use watchman to detect changes to trigger further invocations
	w := watcher.NewWatchman(wd)
	if err := w.Start(); err != nil {
		return fmt.Errorf("failed to start the watcher: %w", err)
	}
	defer w.Close()

	ctx, t := p.tracer.Start(ctx, "Configure.Subscribe", trace.WithAttributes(
		traceAttr.String("mode", mode),
		traceAttr.StringSlice("languages", p.languageKeys),
		traceAttr.StringSlice("excludes", excludes),
		traceAttr.StringSlice("args", args),
	))
	defer t.End()

	// Subscribe to further changes
	for cs, err := range w.Subscribe(ctx, "aspect-configure-watch") {
		if err != nil {
			return fmt.Errorf("failed to get next event: %w", err)
		}

		_, t := p.tracer.Start(ctx, "Configure.Subscribe.Trigger")

		// Enter into a state to discard supirious changes caused by potential file atime
		// updates by gazelle languages.
		if err := w.StateEnter("aspect-configure-watch"); err != nil {
			return fmt.Errorf("failed to enter watch state: %w", err)
		}

		fmt.Fprintf(p.Streams.Stdout, "Detected %d changes in %v: %v\n", len(cs.Paths), wd, cs.Paths)

		// The directories that have changed which gazelle should update.
		// This assumes all enabled gazelle languages support incremental updates.
		changedDirs := make([]string, 0, len(cs.Paths))
		for _, p := range cs.Paths {
			changedDirs = append(changedDirs, path.Dir(p))
		}

		// Run gazelle
		stats, err := RunGazelleFixUpdate(wd, p.InstantiateLanguages(), append(fixArgs, changedDirs...))
		if err != nil {
			return fmt.Errorf("failed to run gazelle fix/update: %w", err)
		}

		// Only output when changes were made, otherwise hopefully the execution was fast enough to be unnoticeable.
		if stats.NumBuildFilesUpdated > 0 {
			fmt.Fprintf(p.Streams.Stdout, "%v/%v BUILD files updated\n", stats.NumBuildFilesUpdated, stats.NumBuildFilesVisited)
		}

		// Leave the state and fast forward the subscription clock.
		if err := w.StateLeave("aspect-configure-watch"); err != nil {
			return fmt.Errorf("failed to leave watch state: %w", err)
		}

		t.End()
	}

	fmt.Fprintf(p.Streams.Stdout, "BUILD file generation --watch exiting...\n")

	return nil
}
