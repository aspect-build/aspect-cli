/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package build

import (
	"fmt"
	"strings"

	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspect/build"
	"aspect.build/cli/pkg/aspect/build/bep"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/hooks"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugins/fix_visibility"
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

// NewDefaultBuildCmd creates a new build cobra command with the default
// dependencies.
func NewDefaultBuildCmd() *cobra.Command {
	return NewBuildCmd(
		ioutils.DefaultStreams,
		bazel.New(),
		bep.NewBESBackend(),
		hooks.New(),
	)
}

// NewBuildCmd creates a new build cobra command.
func NewBuildCmd(
	streams ioutils.Streams,
	bzl bazel.Spawner,
	besBackend bep.BESBackend,
	hooks *hooks.Hooks,
) *cobra.Command {
	// TODO(f0rmiga): this should also be part of the plugin design, as
	// registering BEP event subscribers should not be hardcoded here.
	var fixVisibilityPlugin build.Plugin = fix_visibility.NewDefaultPlugin()
	besBackend.RegisterSubscriber(fixVisibilityPlugin.BEPEventCallback)
	hooks.RegisterPostBuild(fixVisibilityPlugin.PostBuildHook)

	b := build.New(streams, bzl, besBackend, hooks)

	cmd := &cobra.Command{
		Use:   "build",
		Short: "Builds the specified targets, using the options.",
		Long: "Invokes bazel build on the specified targets. " +
			"See 'bazel help target-syntax' for details and examples on how to specify targets to build.",
		RunE: func(cmd *cobra.Command, args []string) (exitErr error) {
			return b.Run(cmd.Context(), cmd, args)
		},
	}

	// Copy over flag metadata from bazel's proto representation
	if f, err := bazel.New().Flags(); err != nil {
		panic("oops")
	} else {
		for k := range f {
			for _, c := range f[k].Commands {
				if c == "build" {
					// check if the flag is boolean-type or string-type
					if *f[k].HasNegativeFlag {
						cmd.Flags().BoolVar(&b.Interesting, k, false, "")
						cmd.Flags().MarkHidden(k)
						cmd.Flags().BoolVar(&b.Interesting, "no"+k, false, "")
						cmd.Flags().MarkHidden("no" + k)
					} else if *f[k].AllowsMultiple {
						var key = MultiString{value: &[]string{}}
						cmd.Flags().VarP(&key, k, "", "")
					} else {
						cmd.Flags().StringVar(&b.StringVar, k, "", "")
						cmd.Flags().MarkHidden(k)

					}
				}
			}
		}
		// fmt.Printf("keys: %v\n", keys)
		// fmt.Printf("Name: %s\n", *f["build"].Name)
		// fmt.Printf("HasNegative: %v\n", *f["build"].AllowsMultiple)
		// fmt.Printf("x: %v\n", f["build"])
	}

	return cmd
}
