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
	"encoding/json"
	"io"
	"os"
	"strings"
	"testing"

	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/aspect-build/silo/workflows/ohno/diagnostic"
	"github.com/spf13/cobra"
)

func runLintHandler(results []*LintResult, args []string) (string, error) {
	stdOutReader, stdOutWriter := io.Pipe()
	stdOut := new(strings.Builder)
	go func() {
		io.Copy(stdOut, stdOutReader)
	}()

	streams := ioutils.Streams{
		Stdout: stdOutWriter,
	}

	lintHandler := LintResultsFileHandler{Streams: streams}

	cmd := &cobra.Command{Use: "lint"}
	AddFlags(cmd.Flags())
	lintHandler.AddFlags(cmd.Flags())

	cmd.SetArgs(args)
	cmd.Execute()

	result := lintHandler.Results(cmd, results)

	stdOutWriter.Close()
	stdOutReader.Close()

	return stdOut.String(), result
}

func patchToDiagnostics(patch []byte, mnemonic string, label string) (diagnostics []*diagnostic.Diagnostic, err error) {
	stdOutReader, stdOutWriter := io.Pipe()
	stdOut := new(strings.Builder)
	go func() {
		io.Copy(stdOut, stdOutReader)
	}()

	streams := ioutils.Streams{
		Stdout: stdOutWriter,
	}

	lintHandler := LintResultsFileHandler{Streams: streams}

	diagnostics, err = lintHandler.patchToDiagnostics(patch, mnemonic, label)

	stdOutWriter.Close()
	stdOutReader.Close()

	return diagnostics, err
}

func createTempJsonFile(name string, t *testing.T) *os.File {
	file, err := os.CreateTemp("", name)

	if err != nil {
		t.Error("Unable to create temp file", err)
	}

	return file
}

func readTempJsonFile(file *os.File, t *testing.T) string {
	fileBytes, err := io.ReadAll(file)
	if err != nil {
		t.Error("Unable to read file", err)
	}
	os.Remove(file.Name())

	// For the reason behind the reformatting see: https://github.com/golang/protobuf/issues/1121
	var terribleProtoJSON interface{}
	err = json.Unmarshal(fileBytes, &terribleProtoJSON)
	if err != nil {
		t.Error("Unable to unmarshal proto json", err)
	}

	formattedJson, err := json.MarshalIndent(terribleProtoJSON, "", "  ")
	if err != nil {
		t.Error("Unable to marshal proto json", err)
	}

	return string(formattedJson)
}
