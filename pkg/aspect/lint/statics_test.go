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

//go:embed testdata/lint_result/ruff_output.txt
var ruff_output string

//go:embed testdata/lint_result/ruff_output_patch.txt
var ruff_output_patch []byte

//go:embed testdata/lint_result/ruff_output_2.txt
var ruff_output_2 string

//go:embed testdata/lint_result/ruff_output_2_patch.txt
var ruff_output_2_patch []byte

//go:embed testdata/lint_result/checkstyle_sarif.txt
var checkstyle_sarif string

//go:embed testdata/lint_result/1_removed_line_report.txt
var one_removed_line_report string

//go:embed testdata/lint_result/1_removed_line_patch.txt
var one_removed_line_patch []byte

//go:embed testdata/lint_result/3_removed_lines_report.txt
var three_removed_lines_report string

//go:embed testdata/lint_result/3_removed_lines_patch.txt
var three_removed_lines_patch []byte

//go:embed testdata/lint_result/removed_lines_first_patch.txt
var removed_lines_first_patch []byte

//go:embed testdata/lint_result/split_removed_lines_patch.txt
var split_removed_lines_patch []byte

func mockResults() []*LintResult {
	var results [13]*LintResult
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

	results[8] = &LintResult{
		Label:    "//monopi/lib/py/os:gpu",
		Mnemonic: "AspectRulesLintRuff",
		ExitCode: 1,
		Report:   ruff_output,
		Patch:    ruff_output_patch,
	}

	results[9] = &LintResult{
		Label:    "//monopi/lib/py/os:gpu",
		Mnemonic: "AspectRulesLintRuff",
		ExitCode: 1,
		Report:   ruff_output_2,
		Patch:    ruff_output_2_patch,
	}

	results[10] = &LintResult{
		Label:    "//apiv2/common/src/main/java/com/vectara/apiv2/common/paging:paging",
		Mnemonic: "AspectRulesLintCheckstyle",
		ExitCode: 1,
		Report:   checkstyle_sarif,
		Patch:    nil,
	}

	results[11] = &LintResult{
		Label:    "//workflows/rosetta/src:src",
		Mnemonic: "AspectRulesLintESLint",
		ExitCode: 1,
		Report:   one_removed_line_report,
		Patch:    one_removed_line_patch,
	}

	results[12] = &LintResult{
		Label:    "//workflows/rosetta/src:src",
		Mnemonic: "AspectRulesLintESLint",
		ExitCode: 1,
		Report:   three_removed_lines_report,
		Patch:    three_removed_lines_patch,
	}

	return results[:]
}
