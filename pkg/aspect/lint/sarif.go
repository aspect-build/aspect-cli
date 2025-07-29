/*
 * Copyright 2024 Aspect Build Systems, Inc.
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
	"bytes"
	"encoding/json"
	"fmt"
	"log"
	"regexp"
	"strings"

	"github.com/reviewdog/errorformat"
	"github.com/reviewdog/errorformat/fmts"
	"github.com/reviewdog/errorformat/writer"
	"github.com/reviewdog/reviewdog/parser"
)

type testStruct struct {
	label    string
	mnemonic string
	report   string
}

func (handler *LintResultsFileHandler) toSarifJsonString(label string, mnemonic string, report string) (sarifJsonString string, err error) {
	regex := regexp.MustCompile(`^{\s+"\$schema":.+sarif`)
	if regex.Match([]byte(report)) {
		return report, nil
	}

	if len(mnemonic) == 0 {
		fmt.Fprintf(handler.Streams.Stderr, "Undefined linter mnemonic for target %s\n", label)
		return "", nil
	}

	var fm []string

	// NB: Switch is on the MNEMONIC declared in rules_lint
	// Helpful link for building custom fm strings: https://vimdoc.sourceforge.net/htmldoc/quickfix.html#errorformat
	// When updating this list, also update the docs here: https://docs.aspect.build/workflows/features/lint#linting
	switch mnemonic {
	case "AspectRulesLintESLint":
		fm = fmts.DefinedFmts()["eslint-compact"].Errorformat
	case "AspectRulesLintFlake8":
		fm = fmts.DefinedFmts()["flake8"].Errorformat
	case "AspectRulesLintPMD":
		// TODO: upstream to https://github.com/reviewdog/errorformat/issues/62
		fm = []string{`%f:%l:\\t%m`}
	case "AspectRulesLintRuff":
		fm = []string{
			`%f:%l:%c: %t%n %m`,
			`%-GFound %n error%.%#`,
			`%-G[*] %n fixable%.%#`,
		}
	case "AspectRulesLintBuf":
		fm = []string{
			`--buf-plugin_out: %f:%l:%c:%m`,
		}
	case "AspectRulesLintVale":
		fm = []string{`%f:%l:%c:%m`}
	case "AspectRulesLintClangTidy":
		fm = []string{
			`%f:%l:%c: %trror: %m`,
			`%f:%l:%c: %tarning: %m`,
			`%-G%m`, // this will ignore any lines that do not match the above 2 lines
			// TODO: Do the other fm's need this ^
		}
	case "AspectRulesLintShellCheck":
		fm = []string{
			`%AIn\ %f\ line\ %l:`,
			`%C%.%#(%trror):\ %m%Z`,
			`%C%.%#(%tarning):\ %m%Z`,
			`%C%.%#`,
		}
	case "AspectRulesLintStylelint":
		fm = []string{
			`%f: line %l\, col %c\, %trror - %m`,
			`%f: line %l\, col %c\, %tarning - %m`,
		}
	default:
		fmt.Fprintf(handler.Streams.Stderr, "No format string for linter mnemonic %s from target %s\n", mnemonic, label)
	}

	if len(fm) == 0 {
		return "", nil
	}
	efm, err := errorformat.NewErrorformat(fm)
	if err != nil {
		return "", err
	}

	var jsonBuffer bytes.Buffer
	var jsonWriter writer.Writer

	var sarifOpt writer.SarifOption
	sarifOpt.ToolName = handler.mnemonicPrettyName(mnemonic)
	jsonWriter, err = writer.NewSarif(&jsonBuffer, sarifOpt)
	if err != nil {
		return "", err
	}

	if jsonWriter, ok := jsonWriter.(writer.BufWriter); ok {
		defer func() {
			if err := jsonWriter.Flush(); err != nil {
				log.Println(err)
			}

			sarifJsonString = jsonBuffer.String()
		}()
	}

	s := efm.NewScanner(strings.NewReader(report))
	for s.Scan() {
		entry := s.Entry()
		if entry.Filename != "" && entry.Text != "" {
			entry.Filename = determineRelativePath(entry.Filename, label)
			if err := jsonWriter.Write(entry); err != nil {
				return "", err
			}
		}
	}

	return sarifJsonString, nil
}

// We expect relative paths when processing lint output and therefore need to convert any absolute paths.
// Assumptions we make when determining the relative paths:
//   - The linter is running on the host, so the path will have an 'execroot' segment
//   - We only lint source files, so there is no 'bazel-bin/<platform>/bin' segment
func determineRelativePath(path string, label string) string {
	if !strings.HasPrefix(path, "/") || !strings.HasPrefix(label, "//") {
		return path
	}

	bazel_package := strings.Split(label[2:], ":")[0]

	// https://regex101.com/r/uMbVHP/1
	re := regexp.MustCompile(`\/execroot\/[^\/]+\/(.*)$`)
	if bazel_package != "" {
		re = regexp.MustCompile(`\/execroot\/[^\/]+\/(` + bazel_package + `\/.*)$`)
	}
	relative_path := re.FindSubmatch([]byte(path))

	if relative_path != nil && len(relative_path) == 2 {
		return string(relative_path[1])
	}

	return path
}

func (handler *LintResultsFileHandler) toSarifJson(sarifJsonString string) (sarifJson parser.SarifJson, err error) {
	if sarifJsonString == "" {
		return parser.SarifJson{}, nil
	}

	err = json.Unmarshal([]byte(sarifJsonString), &sarifJson)

	return sarifJson, err
}
