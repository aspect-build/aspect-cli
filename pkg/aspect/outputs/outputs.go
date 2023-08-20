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

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
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

func (runner *Outputs) Run(_ context.Context, _ *cobra.Command, args []string) error {
	nonFlags, bazelFlags, err := bazel.ParseOutBazelFlags("aquery", args)
	if err != nil {
		return err
	}

	// Strip any leading -- that can come from ParseOutBazelFlags
	if len(nonFlags) > 0 && nonFlags[0] == "--" {
		nonFlags = nonFlags[1:]
	}

	if len(nonFlags) < 1 {
		return fmt.Errorf("a query expression is required as the first argument to outputs command")
	}

	query := nonFlags[0]

	var mnemonicFilter string
	if len(nonFlags) == 2 {
		mnemonicFilter = nonFlags[1]
	} else if len(nonFlags) > 2 {
		return fmt.Errorf("expecting a maximum of 2 arguments to outputs command but got %v", len(nonFlags))
	}

	agc, err := runner.bzl.AQuery(query, bazelFlags)
	if err != nil {
		return err
	}
	outs := bazel.ParseOutputs(agc)

	// Special case pseudo-mnemonic indicating we should compute an overall hash
	// for any executables in the aquery result
	if mnemonicFilter == "ExecutableHash" {
		hashes, err := gatherExecutableHashes(outs)
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
