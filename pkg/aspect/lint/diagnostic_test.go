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

package lint

import (
	"io"
	"strings"
	"testing"

	"github.com/aspect-build/aspect-cli/pkg/aspect/lint/diagnostic"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	. "github.com/onsi/gomega"
)

func TestDiagnostics(t *testing.T) {
	t.Run("diagnostics get the correct severity", func(t *testing.T) {
		g := NewGomegaWithT(t)
		stdOutReader, stdOutWriter := io.Pipe()
		stdOut := new(strings.Builder)
		go func() {
			io.Copy(stdOut, stdOutReader)
		}()

		streams := ioutils.Streams{
			Stdout: stdOutWriter,
		}

		lintHandler := LintResultsFileHandler{Streams: streams}

		sarifJsonString, _ := lintHandler.toSarifJsonString("//workflows/marvin/domain:domain_tests_typings", "AspectRulesLintESLint", eslint_output)
		sarifJson, _ := lintHandler.toSarifJson(sarifJsonString)
		diagnostics := lintHandler.sarifToDiagnostics(sarifJson, "//workflows/marvin/domain:domain_tests_typings")

		stdOutWriter.Close()
		stdOutReader.Close()

		g.Expect(diagnostics[0].Severity).To(Equal(diagnostic.Severity_WARNING))
		g.Expect(diagnostics[1].Severity).To(Equal(diagnostic.Severity_ERROR))
		g.Expect(diagnostics[2].Severity).To(Equal(diagnostic.Severity_WARNING))
		g.Expect(diagnostics[3].Severity).To(Equal(diagnostic.Severity_WARNING))
		g.Expect(diagnostics[4].Severity).To(Equal(diagnostic.Severity_ERROR))
	})

	t.Run("sarif diagnostics get a label added to baggage", func(t *testing.T) {
		g := NewGomegaWithT(t)
		stdOutReader, stdOutWriter := io.Pipe()
		stdOut := new(strings.Builder)
		go func() {
			io.Copy(stdOut, stdOutReader)
		}()

		streams := ioutils.Streams{
			Stdout: stdOutWriter,
		}

		lintHandler := LintResultsFileHandler{Streams: streams}

		sarifJsonString, _ := lintHandler.toSarifJsonString("//workflows/marvin/domain:domain_tests_typings", "AspectRulesLintESLint", eslint_output)
		sarifJson, _ := lintHandler.toSarifJson(sarifJsonString)
		diagnostics := lintHandler.sarifToDiagnostics(sarifJson, "//workflows/marvin/domain:domain_tests_typings")

		stdOutWriter.Close()
		stdOutReader.Close()

		g.Expect(diagnostics[0].Baggage["label"]).To(Equal("//workflows/marvin/domain:domain_tests_typings"))
		g.Expect(diagnostics[1].Baggage["label"]).To(Equal("//workflows/marvin/domain:domain_tests_typings"))
		g.Expect(diagnostics[2].Baggage["label"]).To(Equal("//workflows/marvin/domain:domain_tests_typings"))
		g.Expect(diagnostics[3].Baggage["label"]).To(Equal("//workflows/marvin/domain:domain_tests_typings"))
		g.Expect(diagnostics[4].Baggage["label"]).To(Equal("//workflows/marvin/domain:domain_tests_typings"))
	})

	t.Run("patch diagnostics get a label added to baggage", func(t *testing.T) {
		g := NewGomegaWithT(t)

		diagnostics, _ := patchToDiagnostics(different_starting_lines, "AspectRulesLintESLint", "//workflows/marvin/domain:domain_tests_typings")

		g.Expect(diagnostics[0].Baggage["label"]).To(Equal("//workflows/marvin/domain:domain_tests_typings"))
	})

	t.Run("patchToDiagnostics: works with different incoming starting - & + lines", func(t *testing.T) {
		g := NewGomegaWithT(t)

		diagnostics, err := patchToDiagnostics(different_starting_lines, "AspectRulesLintESLint", "//workflows/marvin/domain:domain_tests_typings")

		g.Expect(err).To(BeNil())
		g.Expect(len(diagnostics)).To(Equal(1))
		g.Expect(diagnostics[0].Help).To(Equal("        flags: readonly (BazelFlag | [string, string] | string)[]"))
		g.Expect(diagnostics[0].Message).To(Equal(""))
		g.Expect(diagnostics[0].GetSourceContent().Name).To(Equal("workflows/rosetta/src/bazel/command.ts"))
		g.Expect(diagnostics[0].Spans[0].Offset).To(Equal(int32(179)))
		g.Expect(diagnostics[0].Spans[0].Height).To(Equal(int32(0)))
	})

	t.Run("patchToDiagnostics: works with multiple changes in the same chunk", func(t *testing.T) {
		g := NewGomegaWithT(t)

		diagnostics, err := patchToDiagnostics(multiple_changes_same_hunk, "AspectRulesLintESLint", "//workflows/marvin/domain:domain_tests_typings")

		g.Expect(err).To(BeNil())
		g.Expect(len(diagnostics)).To(Equal(1))
		g.Expect(diagnostics[0].Help).To(Equal(`    private attempts: Result<ProcessOutput, ProcessOutput>[] = [];
    private invocations = new Set<string>();

    private retryCodes: ReadonlySet<BazelExitCode> = DEFAULT_RETRY_CODES;
    private retryDelay = 3000;
    private retryAttempts = 3;`))
		g.Expect(diagnostics[0].Message).To(Equal(""))
		g.Expect(diagnostics[0].GetSourceContent().Name).To(Equal("workflows/rosetta/src/bazel/executor.ts"))
		g.Expect(diagnostics[0].Spans[0].Offset).To(Equal(int32(87)))
		g.Expect(diagnostics[0].Spans[0].Height).To(Equal(int32(5)))
	})

	t.Run("patchToDiagnostics: works with spaced adds and grouped removes", func(t *testing.T) {
		g := NewGomegaWithT(t)

		diagnostics, err := patchToDiagnostics(spaced_add_grouped_remove, "AspectRulesLintESLint", "//workflows/marvin/domain:domain_tests_typings")

		g.Expect(err).To(BeNil())
		g.Expect(len(diagnostics)).To(Equal(1))
		g.Expect(diagnostics[0].Help).To(Equal(`import { maybe, none, Option } from '../../../../tslibs/result';
import { ChangedFiles } from '../../../git-state';
import { AsyncResult,DiagnosticInput, SeveritySchema } from '../../../ohno';
import { BazelCommand, BazelExitCode, BazelServerDirectories } from '../bazel';
import { LabelSchema } from '../configuration/bazel.schema';
import { Logger } from '../logger';
import { TMP_DIR } from '../utils';
import { BazelTaskConfigurationSchema, BazelTaskRef } from './bazel.task';
import { TaskType, TaskTypeSchemaWithType } from './domain/task-type';
import { TaskEvent, TaskEventPayload } from './task-events';
import { TaskOutcome } from './task-outcome';`))
		g.Expect(diagnostics[0].Message).To(Equal(""))
		g.Expect(diagnostics[0].GetSourceContent().Name).To(Equal("workflows/rosetta/src/tasks/lint.task.ts"))
		g.Expect(diagnostics[0].Spans[0].Offset).To(Equal(int32(5)))
		g.Expect(diagnostics[0].Spans[0].Height).To(Equal(int32(10)))
	})

	t.Run("patchToDiagnostics: multiple patches in a file", func(t *testing.T) {
		g := NewGomegaWithT(t)

		diagnostics, err := patchToDiagnostics(multi_issue_patch, "AspectRulesLintESLint", "//workflows/rosetta:bin")

		g.Expect(err).To(BeNil())
		g.Expect(len(diagnostics)).To(Equal(4))
		g.Expect(diagnostics[0].Help).To(Equal(`import { maybe, none, Option } from '../../../../tslibs/result';
import { ChangedFiles } from '../../../git-state';
import { AsyncResult,DiagnosticInput, SeveritySchema } from '../../../ohno';
import { BazelCommand, BazelExitCode, BazelServerDirectories } from '../bazel';
import { LabelSchema } from '../configuration/bazel.schema';
import { Logger } from '../logger';
import { TMP_DIR } from '../utils';
import { BazelTaskConfigurationSchema, BazelTaskRef } from './bazel.task';
import { TaskType, TaskTypeSchemaWithType } from './domain/task-type';
import { TaskEvent, TaskEventPayload } from './task-events';
import { TaskOutcome } from './task-outcome';`))
		g.Expect(diagnostics[0].Message).To(Equal(""))
		g.Expect(diagnostics[0].GetSourceContent().Name).To(Equal("workflows/rosetta/src/tasks/lint.task.ts"))
		g.Expect(diagnostics[0].Spans[0].Offset).To(Equal(int32(5)))
		g.Expect(diagnostics[0].Spans[0].Height).To(Equal(int32(10)))

		g.Expect(diagnostics[1].Help).To(Equal(`            const maybeChangedFiles = ChangedFiles.FromHeadCommit();`))
		g.Expect(diagnostics[1].Message).To(Equal(""))
		g.Expect(diagnostics[1].GetSourceContent().Name).To(Equal("workflows/rosetta/src/tasks/lint.task.ts"))
		g.Expect(diagnostics[1].Spans[0].Offset).To(Equal(int32(187)))
		g.Expect(diagnostics[1].Spans[0].Height).To(Equal(int32(0)))

		g.Expect(diagnostics[2].Help).To(Equal(`            const changedFiles: ChangedFiles = maybeChangedFiles.unwrap();`))
		g.Expect(diagnostics[2].Message).To(Equal(""))
		g.Expect(diagnostics[2].GetSourceContent().Name).To(Equal("workflows/rosetta/src/tasks/lint.task.ts"))
		g.Expect(diagnostics[2].Spans[0].Offset).To(Equal(int32(199)))
		g.Expect(diagnostics[2].Spans[0].Height).To(Equal(int32(0)))

		g.Expect(diagnostics[3].Help).To(Equal(`        const aIsFromChanged = diagnosticIsFromChangedLine(a, changedFiles);
        const bIsFromChanged = diagnosticIsFromChangedLine(b, changedFiles);`))
		g.Expect(diagnostics[3].Message).To(Equal(""))
		g.Expect(diagnostics[3].GetSourceContent().Name).To(Equal("workflows/rosetta/src/tasks/lint.task.ts"))
		g.Expect(diagnostics[3].Spans[0].Offset).To(Equal(int32(347)))
		g.Expect(diagnostics[3].Spans[0].Height).To(Equal(int32(1)))
	})

	t.Run("creates clang tidy diagnostics with the correct paths", func(t *testing.T) {
		g := NewGomegaWithT(t)
		stdOutReader, stdOutWriter := io.Pipe()
		stdOut := new(strings.Builder)
		go func() {
			io.Copy(stdOut, stdOutReader)
		}()

		streams := ioutils.Streams{
			Stdout: stdOutWriter,
		}

		lintHandler := LintResultsFileHandler{Streams: streams}

		sarifJsonString, _ := lintHandler.toSarifJsonString("//speller/announce:announce", "AspectRulesLintClangTidy", clang_tidy_output)
		sarifJson, _ := lintHandler.toSarifJson(sarifJsonString)
		diagnostics := lintHandler.sarifToDiagnostics(sarifJson, "//speller/announce:announce")

		stdOutWriter.Close()
		stdOutReader.Close()

		g.Expect(len(diagnostics)).To(Equal(2))
		g.Expect(diagnostics[0].GetSourceContent().Name).To(Equal("speller/announce/announce.cc"))
		g.Expect(diagnostics[1].GetSourceContent().Name).To(Equal("speller/announce/announce.cc"))
	})
}
