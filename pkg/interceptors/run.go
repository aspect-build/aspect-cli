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

package interceptors

import (
	"context"

	"github.com/spf13/cobra"
)

// RunEFn matches the cobra command.RunE signature.
type RunEFn func(cmd *cobra.Command, args []string) error

// RunEContextFn is the signature based on RunEFn that injects the context as
// an argument.
type RunEContextFn func(ctx context.Context, cmd *cobra.Command, args []string) error

// Interceptor represents an interceptor in the CLI command chain. It's
// represented as a function signature.
type Interceptor func(ctx context.Context, cmd *cobra.Command, args []string, next RunEContextFn) error

// Run returns a function that matches the cobra RunE signature. It assembles
// the interceptors and main command to be run in the correct sequence.
func Run(interceptors []Interceptor, fn RunEContextFn) RunEFn {
	return func(cmd *cobra.Command, args []string) error {
		current := fn
		for i := len(interceptors) - 1; i >= 0; i-- {
			interceptor := interceptors[i]
			next := current
			current = func(ctx context.Context, cmd *cobra.Command, args []string) error {
				return interceptor(ctx, cmd, args, next)
			}
		}
		return current(cmd.Context(), cmd, args)
	}
}
