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
	"strings"

	"github.com/aspect-build/silo/workflows/ohno/diagnostic"
	"github.com/reviewdog/reviewdog/parser"
	godiff "github.com/sourcegraph/go-diff/diff"
)

func (handler *LintResultsFileHandler) sarifToDiagnostics(sarif parser.SarifJson, label string) []*diagnostic.Diagnostic {
	var diagnostics []*diagnostic.Diagnostic
	for _, run := range sarif.Runs {
		for _, result := range run.Results {
			for _, location := range result.Locations {
				rdfRange := location.PhysicalLocation.Region.GetRdfRange()
				// TODO: The else case likely means it applies to the whole file. Need to account for that
				if rdfRange != nil {
					diagnostics = append(diagnostics, &diagnostic.Diagnostic{
						Message:  result.Message.Text,
						Severity: toSeverity(result.Level),
						Source: &diagnostic.Diagnostic_SourceContent{
							SourceContent: &diagnostic.SourceContent{
								Name: location.PhysicalLocation.ArtifactLocation.URI,
							},
						},
						Spans: []*diagnostic.Span{{
							Offset: int32(location.PhysicalLocation.Region.GetRdfRange().Start.Line),
						}},
						Title:   run.Tool.Driver.Name + " found an issue",
						Type:    diagnostic.DiagnosticType_FILE,
						Baggage: map[string]string{"label": label},
					})
				}
			}
		}
	}

	return diagnostics
}

func toSeverity(sarifLevel string) diagnostic.Severity {
	if strings.ToLower(sarifLevel) == "error" {
		return diagnostic.Severity_ERROR
	}
	return diagnostic.Severity_WARNING
}

type Hunk struct {
	Path string
	Span diagnostic.Span
	Body string
}

func (handler *LintResultsFileHandler) patchToDiagnostics(patch []byte, mnemonic string, label string) (diagnostics []*diagnostic.Diagnostic, err error) {
	hunks, err := patchToHunks(patch)
	if err != nil {
		return diagnostics, err
	}

	if hunks == nil {
		return make([]*diagnostic.Diagnostic, 0), nil
	}

	toolName := "Lint"
	if mnemonic != "" {
		toolName = handler.mnemonicPrettyName(mnemonic)
	}

	diagnostics = make([]*diagnostic.Diagnostic, len(hunks))
	//nolint:copylocks // Single threaded
	for i, hunk := range hunks {
		diagnostics[i] = &diagnostic.Diagnostic{
			Severity: diagnostic.Severity_ERROR,
			Source: &diagnostic.Diagnostic_SourceContent{
				SourceContent: &diagnostic.SourceContent{
					Name: hunk.Path,
				},
			},
			Spans: []*diagnostic.Span{{
				Offset: int32(hunk.Span.Offset),
				Height: int32(hunk.Span.Height),
			}},
			Title:   toolName + " provided a suggestion",
			Help:    hunk.Body,
			Type:    diagnostic.DiagnosticType_FILE,
			Baggage: map[string]string{"label": label},
		}
	}

	return diagnostics, nil
}

func baseHunk(NewName string) Hunk {
	return Hunk{
		Path: strings.Replace(NewName, "b/", "", 1),
		Span: diagnostic.Span{
			Offset: 0,
			Height: 0,
		},
		Body: "",
	}
}

func patchToHunks(patch []byte) (hunks []Hunk, err error) {
	diffs, err := godiff.ParseMultiFileDiff(patch)
	if err != nil {
		return hunks, err
	}

	for _, diff := range diffs {
		hunk := baseHunk(diff.NewName)
		for _, diffHunk := range diff.Hunks {
			lineNumber := diffHunk.OrigStartLine
			var potentialBody []string
			var body []string
			var changesStarted bool = false
			var potentialHeight int32 = 0
			for _, line := range strings.Split(strings.TrimSuffix(string(diffHunk.Body), "\n"), "\n") {
				if !strings.HasPrefix(line, "+") && changesStarted {
					potentialHeight = potentialHeight + 1
				}
				if (strings.HasPrefix(line, "-") || strings.HasPrefix(line, "+")) && !changesStarted {
					hunk.Span.Offset = int32(lineNumber)
					if strings.HasPrefix(line, "-") {
						hunk.Span.Height = 1
					}
					changesStarted = true
				}
				if strings.HasPrefix(line, "+") {
					body = append(body, potentialBody...)
					body = append(body, strings.Replace(line, "+", "", 1))
					potentialBody = make([]string, 0)
					hunk.Span.Height = int32(hunk.Span.Height) + int32(potentialHeight)
					potentialHeight = 0
				} else if strings.HasPrefix(line, "-") {
					body = append(body, potentialBody...)
					potentialBody = make([]string, 0)
					hunk.Span.Height = int32(hunk.Span.Height) + int32(potentialHeight)
					potentialHeight = 0
				} else if changesStarted {
					potentialBody = append(potentialBody, strings.Replace(line, " ", "", 1))
				}

				if !strings.HasPrefix(line, "+") {
					lineNumber = lineNumber + 1
				}
			}

			hunk.Span.Height = hunk.Span.Height - 1
			hunk.Body = strings.Join(body, "\n")

			//nolint:copylocks // Single threaded
			hunks = append(hunks, hunk)
		}
	}

	return hunks, nil
}
