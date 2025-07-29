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
	"testing"

	"gotest.tools/v3/golden"

	. "github.com/onsi/gomega"
)

const (
	GOLDEN_DIAGNOSTICS_JSON_PATH = "golden_files/mockResults_diagnostics.json"
	GOLDEN_SARIF_JSON_PATH       = "golden_files/mockResults_sarif.json"
)

func TestLintHandler(t *testing.T) {
	t.Run("creates a diagnostics file", func(t *testing.T) {
		g := NewGomegaWithT(t)

		tempFile := createTempJsonFile("diagnostics*.json", t)

		mockResults := mockResults()

		stdOut, err := runLintHandler(mockResults, []string{"lint", "--machine", "--lint_diagnostics_file=" + tempFile.Name()})
		g.Expect(err).To(BeNil())
		if stdOut != "" {
			t.Error("Unexpected output", stdOut)
		}

		generatedFile := readTempJsonFile(tempFile, t)
		golden.Assert(t, generatedFile, GOLDEN_DIAGNOSTICS_JSON_PATH)
	})

	t.Run("creates a sarif file", func(t *testing.T) {
		g := NewGomegaWithT(t)

		tempFile := createTempJsonFile("sarif*.json", t)

		mockResults := mockResults()

		stdOut, err := runLintHandler(mockResults, []string{"lint", "--machine", "--lint_sarif_file=" + tempFile.Name()})
		g.Expect(err).To(BeNil())
		if stdOut != "" {
			t.Error("Unexpected output", stdOut)
		}

		generatedFile := readTempJsonFile(tempFile, t)
		golden.Assert(t, generatedFile, GOLDEN_SARIF_JSON_PATH)
	})

	t.Run("creates a sarif file and a diagnostics file", func(t *testing.T) {
		g := NewGomegaWithT(t)

		sarifTempFile := createTempJsonFile("sarif*.json", t)
		diagnosticsTempFile := createTempJsonFile("diagnostics*.json", t)

		mockResults := mockResults()

		stdOut, err := runLintHandler(mockResults, []string{"lint", "--machine", "--lint_sarif_file=" + sarifTempFile.Name(), "--lint_diagnostics_file=" + diagnosticsTempFile.Name()})
		g.Expect(err).To(BeNil())
		if stdOut != "" {
			t.Error("Unexpected output", stdOut)
		}

		sarifGeneratedFile := readTempJsonFile(sarifTempFile, t)
		golden.Assert(t, sarifGeneratedFile, GOLDEN_SARIF_JSON_PATH)

		diagnosticsGeneratedFile := readTempJsonFile(diagnosticsTempFile, t)
		golden.Assert(t, diagnosticsGeneratedFile, GOLDEN_DIAGNOSTICS_JSON_PATH)
	})
}
