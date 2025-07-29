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
	_ "embed"
)

//go:embed testdata/lint_result/imports_should_be_sorted_report.txt
var imports_should_be_sorted_report string

//go:embed testdata/lint_result/imports_should_be_sorted_patch.txt
var imports_should_be_sorted_patch []byte

//go:embed testdata/lint_result/multiple_syntax_before_single_syntax_report.txt
var multiple_syntax_before_single_syntax_report string

//go:embed testdata/lint_result/multiple_syntax_before_single_syntax_patch.txt
var multiple_syntax_before_single_syntax_patch []byte

//go:embed testdata/lint_result/different_starting_lines.txt
var different_starting_lines []byte

//go:embed testdata/lint_result/multiple_changes_same_hunk.txt
var multiple_changes_same_hunk []byte

//go:embed testdata/lint_result/spaced_add_grouped_remove.txt
var spaced_add_grouped_remove []byte

//go:embed testdata/lint_result/multi_issue_patch.txt
var multi_issue_patch []byte

//go:embed testdata/lint_result/eslint_output.txt
var eslint_output string

//go:embed testdata/lint_result/clang_tidy_output.txt
var clang_tidy_output string

//go:embed testdata/lint_result/shellcheck_output_1.txt
var shellcheck_output_1 string

//go:embed testdata/lint_result/shellcheck_output_2.txt
var shellcheck_output_2 string

//go:embed testdata/lint_result/stylelint_output.txt
var stylelint_output string

func mockResults() []*LintResult {
	var results [8]*LintResult
	results[0] = &LintResult{
		Label:    "//workflows/marvin/domain:domain_tests",
		Mnemonic: "AspectRulesLintESLint",
		ExitCode: 0,
		Report:   "",
		Patch:    nil,
	}

	results[1] = &LintResult{
		Label:    "//workflows/marvin/domain:domain_tests_typings",
		Mnemonic: "AspectRulesLintESLint",
		ExitCode: 1,
		Report:   imports_should_be_sorted_report,
		Patch:    imports_should_be_sorted_patch,
	}

	results[2] = &LintResult{
		Label:    "//workflows/marvin/domain:domain",
		Mnemonic: "AspectRulesLintESLint",
		ExitCode: 0,
		Report:   "",
		Patch:    nil,
	}

	results[3] = &LintResult{
		Label:    "//workflows/marvin/domain:domain_typings",
		Mnemonic: "AspectRulesLintESLint",
		ExitCode: 1,
		Report:   multiple_syntax_before_single_syntax_report,
		Patch:    multiple_syntax_before_single_syntax_patch,
	}

	results[4] = &LintResult{
		Label:    "//speller/announce:announce",
		Mnemonic: "AspectRulesLintClangTidy",
		ExitCode: 1,
		Report:   clang_tidy_output,
		Patch:    nil,
	}

	results[5] = &LintResult{
		Label:    "//docs:docs_delivery",
		Mnemonic: "AspectRulesLintShellCheck",
		ExitCode: 1,
		Report:   shellcheck_output_1,
		Patch:    nil,
	}

	results[6] = &LintResult{
		Label:    "//integration_tests:shell",
		Mnemonic: "AspectRulesLintShellCheck",
		ExitCode: 1,
		Report:   shellcheck_output_2,
		Patch:    nil,
	}

	results[7] = &LintResult{
		Label:    "//src:css",
		Mnemonic: "AspectRulesLintStylelint",
		ExitCode: 1,
		Report:   stylelint_output,
		Patch:    nil,
	}

	return results[:]
}
