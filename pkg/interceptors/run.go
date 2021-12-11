/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
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
		for i := len(interceptors) - 1; i > 0; i-- {
			j := i
			next := current
			current = func(ctx context.Context, cmd *cobra.Command, args []string) error {
				return interceptors[j](ctx, cmd, args, next)
			}
		}
		return interceptors[0](cmd.Context(), cmd, args, current)
	}
}
