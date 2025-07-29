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
	"encoding/json"
	"os"
	"path/filepath"
	"strings"

	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/aspect-build/silo/workflows/ohno/diagnostic"
	"github.com/fatih/color"
	"github.com/reviewdog/reviewdog/parser"
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
	"google.golang.org/protobuf/encoding/protojson"
)

type LintResultsFileHandler struct {
	ioutils.Streams
}

var _ LintResultsHandler = (*LintResultsFileHandler)(nil)

type LintResultOutput struct {
	DiagnosticsFile string
	SarifFile       string
}

func (handler *LintResultsFileHandler) AddFlags(flags *pflag.FlagSet) {
	flags.String("lint_diagnostics_file", "", "Path for writing lint result diagnostics")
	flags.MarkHidden("lint_diagnostics_file")
	flags.String("lint_sarif_file", "", "Path for writing lint result SARIF JSON")
	flags.MarkHidden("lint_sarif_file")
}

func (handler *LintResultsFileHandler) mnemonicPrettyName(mnemonic string) string {
	return strings.Replace(mnemonic, "AspectRulesLint", "", 1)
}

func (handler *LintResultsFileHandler) Results(cmd *cobra.Command, results []*LintResult) (err error) {
	machine, _ := cmd.Flags().GetBool("machine")
	if !machine {
		// Without the machine flag we will get human readable output, which we cannot process.
		return nil
	}

	resultOutput := processFlags(cmd)

	allDiagnostics := &diagnostic.Diagnostics{}
	var allSarif []parser.SarifJson

	for _, r := range results {
		if len(r.Report) > 0 {
			// Sarif
			sarifString, err := handler.toSarifJsonString(r.Label, r.Mnemonic, r.Report)
			if err != nil {
				return err
			}
			sarifJson, err := handler.toSarifJson(sarifString)
			if err != nil {
				return err
			}
			allSarif = append(allSarif, sarifJson)

			// Diagnostics
			diagnostics := handler.sarifToDiagnostics(sarifJson, r.Label)
			allDiagnostics.Diagnostics = append(allDiagnostics.Diagnostics, diagnostics...)

			if r.Patch != nil {
				patchDiagnostics, err := handler.patchToDiagnostics(r.Patch, r.Mnemonic, r.Label)
				if err != nil {
					return err
				}

				allDiagnostics.Diagnostics = append(allDiagnostics.Diagnostics, patchDiagnostics...)
			}
		}

		if r.Patch != nil {
			color.New(color.FgHiGreen).Printf("Patch for %s from linter mnemonic %s\n", r.Label, r.Mnemonic)
		}
	}

	if resultOutput.SarifFile != "" {
		sarifJson, err := json.Marshal(allSarif)
		if err != nil {
			return err
		}

		dir := filepath.Dir(resultOutput.SarifFile)
		err = os.MkdirAll(dir, os.ModePerm)
		if err != nil {
			return err
		}

		err = os.WriteFile(resultOutput.SarifFile, sarifJson, 0755)
		if err != nil {
			return err
		}
	}

	if resultOutput.DiagnosticsFile != "" {
		diagnosticsJson, err := protojson.MarshalOptions{
			UseProtoNames:  true,
			UseEnumNumbers: true,
			Multiline:      true,
		}.Marshal(allDiagnostics)
		if err != nil {
			return err
		}

		dir := filepath.Dir(resultOutput.DiagnosticsFile)
		err = os.MkdirAll(dir, os.ModePerm)
		if err != nil {
			return err
		}

		err = os.WriteFile(resultOutput.DiagnosticsFile, diagnosticsJson, 0755)
		if err != nil {
			return err
		}
	}

	return nil
}

func processFlags(cmd *cobra.Command) LintResultOutput {
	resultOutput := LintResultOutput{}

	if diagnosticsFile, _ := cmd.Flags().GetString("lint_diagnostics_file"); diagnosticsFile != "" {
		resultOutput.DiagnosticsFile = diagnosticsFile
	}

	if sarifFile, _ := cmd.Flags().GetString("lint_sarif_file"); sarifFile != "" {
		resultOutput.SarifFile = sarifFile
	}

	return resultOutput
}
