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

	"github.com/aspect-build/aspect-cli/pkg/aspect/lint/diagnostic"
	"github.com/reviewdog/reviewdog/parser"
	"github.com/sourcegraph/go-diff/diff"
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
								Name: determineRelativePath(location.PhysicalLocation.ArtifactLocation.URI, label),
							},
						},
						Spans: []*diagnostic.Span{{
							Offset: int32(location.PhysicalLocation.Region.GetRdfRange().Start.Line),
						}},
						Title: run.Tool.Driver.Name + " found an issue",
						Type:  diagnostic.DiagnosticType_FILE,
						Baggage: map[string]string{
							"label":            label,
							"lint_result_type": "annotation",
						},
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
			Title: toolName + " provided a suggestion",
			Help:  hunk.Body,
			Type:  diagnostic.DiagnosticType_FILE,
			Baggage: map[string]string{
				"label":            label,
				"lint_result_type": "suggestion",
			},
		}
	}

	return diagnostics, nil
}

func patchToHunks(patch []byte) (hunks []Hunk, err error) {
	diffs, err := diff.ParseMultiFileDiff(patch)
	if err != nil {
		return nil, err
	}

	for _, fileDiff := range diffs {
		for _, diffHunk := range fileDiff.Hunks {
			lines := strings.Split(string(diffHunk.Body), "\n")

			// --- Pass 1: Find the start and end indices of the change block. ---
			firstChangeIndex, lastChangeIndex := -1, -1
			for i, line := range lines {
				if len(line) > 0 && (line[0] == '+' || line[0] == '-') {
					if firstChangeIndex == -1 {
						firstChangeIndex = i
					}
					lastChangeIndex = i
				}
			}

			// If the hunk contains no changes, skip it.
			if firstChangeIndex == -1 {
				continue
			}

			// --- Pass 2: Calculate file span and build the suggestion body. ---
			var startLineInFile, endLineInFile int32 = -1, -1
			var bodyLines []string
			var foundAddition bool
			currentLineInFile := int32(diffHunk.OrigStartLine)

			// The suggestion body starts from the first non-deletion line within the change block.
			bodyStartIndex := -1
			for i := firstChangeIndex; i <= lastChangeIndex; i++ {
				if len(lines[i]) > 0 && lines[i][0] != '-' {
					bodyStartIndex = i
					break
				}
			}

			for i, line := range lines {
				if len(line) == 0 {
					continue
				}

				// Lines not starting with '+' existed in the original file and advance the line counter.
				if line[0] != '+' {
					// If this line is part of the change block, it defines the span of the change.
					if i >= firstChangeIndex && i <= lastChangeIndex {
						if startLineInFile == -1 {
							startLineInFile = currentLineInFile
						}
						endLineInFile = currentLineInFile
					}
					currentLineInFile++
				} else {
					foundAddition = true
				}

				// If we have found where the body starts, collect all non-deletion lines.
				if bodyStartIndex != -1 && i >= bodyStartIndex && i <= lastChangeIndex && line[0] != '-' {
					bodyLines = append(bodyLines, line[1:])
				}
			}

			// A pure addition has no span in the original file. The "span" is a zero-height
			// insertion point located after the last line preceding the addition.
			if foundAddition && startLineInFile == -1 {
				startLineInFile = currentLineInFile - 1
				endLineInFile = startLineInFile
			}

			hunks = append(hunks, Hunk{
				Path: strings.TrimPrefix(fileDiff.NewName, "b/"),
				Span: diagnostic.Span{
					Offset: startLineInFile,
					Height: endLineInFile - startLineInFile,
				},
				Body: strings.Join(bodyLines, "\n"),
			})
		}
	}

	return hunks, nil
}
