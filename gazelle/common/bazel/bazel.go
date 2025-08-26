package bazel

import (
	"os"

	"github.com/aspect-build/aspect-cli/gazelle/common/bazel/workspace"
)

var workingDirectory string

func FindWorkspaceDirectory() string {
	if workingDirectory == "" {
		// Support running cli via `bazel run`
		workingDirectory = os.Getenv("BUILD_WORKING_DIRECTORY")

		// Fallback to CWD
		if workingDirectory == "" {
			wd, err := os.Getwd()
			if err != nil {
				panic(err)
			}
			workingDirectory = wd
		}
	}
	finder := workspace.DefaultFinder
	wr, err := finder.Find(workingDirectory)
	if err != nil {
		return ""
	}
	return wr
}
