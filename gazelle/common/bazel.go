package gazelle

import (
	"os"
	"path"

	"github.com/bazelbuild/bazel-gazelle/config"
)

var (
	// A set of already-seen Bazel packages so we avoid doing
	// disk IO over and over to determine if a directory is a Bazel package.
	isPackageCache = make(map[string]bool)
)

// IsBazelPackage determines if the directory is a Bazel package by probing for
// the existence of a known BUILD file name.
func IsBazelPackage(config *config.Config, dir string) bool {
	return HasBUILDFile(config.ValidBuildFileNames, dir)
}

func HasBUILDFile(buildFileNames []string, dir string) bool {
	if isPkg, cached := isPackageCache[dir]; cached {
		return isPkg
	}

	for _, buildFilename := range buildFileNames {
		buildPath := path.Join(dir, buildFilename)
		if _, err := os.Stat(buildPath); err == nil {
			isPackageCache[dir] = true
			return true
		}
	}
	isPackageCache[dir] = false
	return false
}
