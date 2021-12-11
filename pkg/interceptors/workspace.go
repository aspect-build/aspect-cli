/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package interceptors

import (
	"context"
	"fmt"
	"os"
	"path"

	"aspect.build/cli/pkg/pathutils"
	"github.com/spf13/cobra"
)

// WorkspaceRootKeyType is a type for the WorkspaceRootKey that avoids collisions.
type WorkspaceRootKeyType bool

// WorkspaceRootKey is the key for the injected workspace root into the context.
const WorkspaceRootKey WorkspaceRootKeyType = true

// WorkspaceRootInterceptor checks that the command is being run inside a Bazel
// workspace and injects the workspace root into the context.
func WorkspaceRootInterceptor() Interceptor {
	return workspaceRootInterceptor(
		os.Getwd,
		pathutils.DefaultWorkspaceFinder,
	)
}

func workspaceRootInterceptor(
	osGetwd func() (dir string, err error),
	workspaceFinder pathutils.WorkspaceFinder,
) Interceptor {
	return func(ctx context.Context, cmd *cobra.Command, args []string, next RunEContextFn) error {
		wd, err := osGetwd()
		if err != nil {
			return fmt.Errorf("failed to run command %q: %w", cmd.Use, err)
		}
		workspacePath, err := workspaceFinder.Find(wd)
		if err != nil {
			return fmt.Errorf("failed to run command %q: %w", cmd.Use, err)
		}
		if workspacePath == "" {
			err = fmt.Errorf("the current working directory %q is not a Bazel workspace", wd)
			return fmt.Errorf("failed to run command %q: %w", cmd.Use, err)
		}
		workspaceRoot := path.Dir(workspacePath)
		ctx = context.WithValue(ctx, WorkspaceRootKey, workspaceRoot)
		return next(ctx, cmd, args)
	}
}
