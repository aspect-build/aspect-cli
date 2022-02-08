/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package interceptors

import (
	"context"
	"fmt"
	"strings"

	"aspect.build/cli/pkg/bazel"
	"github.com/spf13/cobra"
)

type Value interface {
	String() string
	Set(string) error
	Type() string
}

type MultiString struct {
	value *[]string
}

func (s *MultiString) Set(value string) error {
	*s.value = append(*s.value, value)
	return nil
}

func (s *MultiString) Type() string {
	return "multiString"
}

func (s *MultiString) String() string {
	return fmt.Sprintf("[ %s ]", strings.Join(*s.value, ", "))
}

func (s *MultiString) First() string {
	return (*s.value)[0]
}

func BazelFlagInterceptor() Interceptor {
	bzl := bazel.New()
	return bazelflagInterceptor(bzl)
}

func bazelflagInterceptor(bzl bazel.Bazel) Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next RunEContextFn) error {
		workspaceRoot := ctx.Value(WorkspaceRootKey).(string)
		bzl.SetWorkspaceRoot(workspaceRoot)

		if bzlFlags, err := bzl.Flags(); err != nil {
			return fmt.Errorf("unable to determine available bazel flags")
		} else {
			for flag := range bzlFlags {
				for _, command := range bzlFlags[flag].Commands {
					if command == cmd.Use {
						if bzlFlags[flag].GetHasNegativeFlag() {
							cmd.Flags().BoolP(flag, bzlFlags[flag].GetAbbreviation(), false, bzlFlags[flag].GetDocumentation())
							cmd.Flags().Bool("no"+flag, false, bzlFlags[flag].GetDocumentation())
						} else if bzlFlags[flag].GetAllowsMultiple() {
							var key = MultiString{value: &[]string{}}
							cmd.Flags().VarP(&key, flag, bzlFlags[flag].GetAbbreviation(), bzlFlags[flag].GetDocumentation())
						} else {
							cmd.Flags().StringP(flag, bzlFlags[flag].GetAbbreviation(), "", bzlFlags[flag].GetDocumentation())
						}
					}
				}
			}
		}

		if !cmd.DisableFlagParsing {
			return fmt.Errorf("flag parsing must be disabled in order to pass through bazel flags")
		}

		// Now that bazel flags have been added to cobra we can turn flag parsing back on and
		// check the incoming flags
		cmd.DisableFlagParsing = false
		if err := cmd.ParseFlags(args); err != nil {
			return err
		}

		return next(ctx, cmd, args)
	}
}
