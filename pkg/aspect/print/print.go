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

package print

import (
	"bytes"
	"context"
	"fmt"
	"strings"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/ioutils"
	"github.com/bazelbuild/buildtools/edit"
)

type Print struct {
	ioutils.Streams
}

func New(streams ioutils.Streams) *Print {
	return &Print{
		Streams: streams,
	}
}

func (v *Print) Run(ctx context.Context, cmd *cobra.Command, args []string) error {
	var stdout bytes.Buffer
	var stderr strings.Builder
	opts := &edit.Options{
		OutWriter: &stdout,
		ErrWriter: &stderr,
		NumIO:     200,
	}
	outputs, err := cmd.Flags().GetStringSlice("output")
	if err != nil {
		return fmt.Errorf("cannot parse output flag: %w", err)
	}

	if ret := edit.Buildozer(opts, append([]string{"print " + strings.Join(outputs, " ")}, args...)); ret != 0 {
		return fmt.Errorf("buildozer exit %d: %s", ret, stderr.String())
	}

	fmt.Print(stdout.String())
	return nil
}
