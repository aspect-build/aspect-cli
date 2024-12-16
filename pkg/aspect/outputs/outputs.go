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

package outputs

import (
	"context"
	"fmt"
	"strings"

	"github.com/spf13/cobra"
	"github.com/spf13/pflag"

	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

type Outputs struct {
	ioutils.Streams
	bzl bazel.Bazel
}

func New(streams ioutils.Streams, bzl bazel.Bazel) *Outputs {
	return &Outputs{
		Streams: streams,
		bzl:     bzl,
	}
}

func AddFlags(flagSet *pflag.FlagSet) {
	flagSet.String("hash_salt", "", "When 'ExecutableHash' is specified, this value will be added as a suffix to every hash")
}

func remove(slice []string, i int) []string {
	return append(slice[:i], slice[i+1:]...)
}

func RemoveCobraFlagsFromArgs(cmd *cobra.Command, args []string) []string {
	cmd.Flags().VisitAll(func(f *pflag.Flag) {
		for i, arg := range args {
			if arg == fmt.Sprintf("--%s=%s", f.Name, f.Value) {
				args = remove(args, i)
				break
			} else if arg == fmt.Sprintf("--%s", f.Name) &&
				len(args)+1 > i &&
				args[i+1] == f.Value.String() {
				args = remove(args, i+1)
				args = remove(args, i)
				break
			}
		}
	})
	return args
}

func (runner *Outputs) Run(_ context.Context, cmd *cobra.Command, args []string) error {
	nonBazelFlags, bazelFlags, err := bazel.SeparateBazelFlags("aquery", args)
	if err != nil {
		return err
	}

	salt := ""
	if cmd != nil {
		nonBazelFlags = RemoveCobraFlagsFromArgs(cmd, nonBazelFlags)
		salt, err = cmd.Flags().GetString("hash_salt")
		if err != nil {
			return err
		}
	}

	// Test to see if the command has been passed the `--query_file` Bazel flag.
	// There is no short hand version of this flag, so the single check is fine.
	hasQueryFileBazelFlag := false
	for _, bazelFlag := range bazelFlags {
		if strings.HasPrefix(bazelFlag, "--query_file") {
			hasQueryFileBazelFlag = true
			break
		}
	}

	// Query will be empty string when supplying a query via a query file.
	// The code in `bzl.Aquery` will filter that.
	var query string
	var mnemonicFilter string
	numNonFlags := len(nonBazelFlags)

	if hasQueryFileBazelFlag {
		// We may have a single arg that is the mnemonic filter, or none.
		if numNonFlags == 1 {
			// The first should be the mnemonic
			mnemonicFilter = nonBazelFlags[0]
		} else if numNonFlags > 1 {
			return fmt.Errorf("expecting a maximum of 1 argument to outputs when using --query_file, got %v", numNonFlags)
		}
	} else {
		// No use of `--query_file`, see what args we do have.
		if numNonFlags < 1 {
			return fmt.Errorf("a query expression is required as the first argument to outputs command")
		}
		query = nonBazelFlags[0]

		if numNonFlags == 2 {
			mnemonicFilter = nonBazelFlags[1]
		} else if numNonFlags > 2 {
			return fmt.Errorf("expecting a maximum of 2 arguments to outputs command but got %v", numNonFlags)
		}
	}

	agc, err := runner.bzl.AQuery(query, bazelFlags)
	if err != nil {
		return err
	}
	outs := bazel.ParseOutputs(agc)

	// Special case pseudo-mnemonic indicating we should compute an overall hash
	// for any executables in the aquery result
	if mnemonicFilter == "ExecutableHash" {
		hashes, err := gatherExecutableHashes(outs, salt)
		if err != nil {
			return err
		}
		for label, hash := range hashes {
			fmt.Fprintf(runner.Stdout, "%s %s\n", label, hash)
		}
		return nil
	}

	for _, a := range outs {
		if len(mnemonicFilter) > 0 {
			if a.Mnemonic == mnemonicFilter {
				fmt.Fprintf(runner.Stdout, "%s\n", a.Path)
			}
		} else {
			fmt.Fprintf(runner.Stdout, "%s %s\n", a.Mnemonic, a.Path)
		}
	}
	return nil
}
