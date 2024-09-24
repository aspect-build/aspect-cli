package gazelle

import (
	"path/filepath"

	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/language"
)

type GazelleWalkFunc func(path string) error

// Must align with patched bazel-gazelle
const ASPECT_WALKSUBDIR = "__aspect:walksubdir"

// Walk the directory of the language.GenerateArgs, optionally recursing into
// subdirectories unlike the files provided in GenerateArgs.RegularFiles.
func GazelleWalkDir(args language.GenerateArgs, walkFunc GazelleWalkFunc) error {
	BazelLog.Tracef("GazelleWalkDir: %s", args.Rel)

	// Source files in the primary directory
	for _, f := range args.RegularFiles {
		// Skip BUILD files
		if args.Config.IsValidBuildFileName(f) {
			continue
		}

		BazelLog.Tracef("GazelleWalkDir RegularFile: %s", f)

		walkErr := walkFunc(f)
		if walkErr != nil && walkErr != filepath.SkipDir {
			return walkErr
		}
	}

	return nil
}
