package gazelle

import (
	"os"
	"path"
)

var (
	// BUILD file names.
	buildFileNames = []string{"BUILD", "BUILD.bazel"}
)

// IsBazelPackage determines if the directory is a Bazel package by probing for
// the existence of a known BUILD file name.
func IsBazelPackage(dir string) bool {
	for _, buildFilename := range buildFileNames {
		buildPath := path.Join(dir, buildFilename)
		if _, err := os.Stat(buildPath); err == nil {
			return true
		}
	}
	return false
}

func isBuildFile(filename string) bool {
	for _, buildFilename := range buildFileNames {
		if filename == buildFilename {
			return true
		}
	}
	return false
}
