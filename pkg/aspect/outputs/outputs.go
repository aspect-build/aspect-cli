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
}

func New(streams ioutils.Streams) *Outputs {
	return &Outputs{
		Streams: streams,
	}
}

func (runner *Outputs) Run(_ context.Context, _ *cobra.Command, args []string) error {
	if len(args) < 1 {
		return fmt.Errorf("a label is required as the first argument to aspect outputs")
	}
	query := args[0]
	var mnemonicFilter string
	if len(args) > 1 {
		mnemonicFilter = args[1]
	}
	bzl, err := bazel.FindFromWd()
	if err != nil {
		return err
	}
	agc, err := bzl.AQuery(query)
	if err != nil {
		return err
	}
	outs := bazel.ParseOutputs(agc)

	// Special case pseudo-mnemonic indicating we should compute an overall hash
	// for any executables in the aquery result
	if mnemonicFilter == "ExecutableHash" {
		hashes, err := printExecutableHashes(outs)
		if err != nil {
			return err
		}
		for label, hash := range hashes {
			fmt.Printf("%s %s\n", label, hash)
		}
		return nil
	}

	for _, a := range outs {
		if len(mnemonicFilter) > 0 {
			if a.Mnemonic == mnemonicFilter {
				fmt.Printf("%s\n", a.Path)
			}
		} else {
			fmt.Printf("%s %s\n", a.Mnemonic, a.Path)
		}
	}
	return nil
}
