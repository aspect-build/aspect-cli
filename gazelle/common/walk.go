package gazelle

import (
	"os"
	"path"
	"path/filepath"

	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/language"
)

type GazelleWalkFunc func(path string) error
type GazelleWalkIgnoreFunc func(path string) bool

// Must align with patched bazel-gazelle
const ASPECT_WALKSUBDIR_PATCHED = "__aspect:walksubdir.patched"
const ASPECT_WALKSUBDIR = "__aspect:walksubdir"

// Walk the directory of the language.GenerateArgs, optionally recursing into
// subdirectories unlike the files provided in GenerateArgs.RegularFiles.
func GazelleWalkDir(args language.GenerateArgs, isIgnored GazelleWalkIgnoreFunc, excludes []string, recurse bool, walkFunc GazelleWalkFunc) error {
	BazelLog.Tracef("GazelleWalkDir: %s", args.Rel)

	// Source files in the primary directory
	for _, f := range args.RegularFiles {
		// Skip BUILD files
		if args.Config.IsValidBuildFileName(f) {
			continue
		}

		if isIgnored(path.Join(args.Rel, f)) {
			BazelLog.Tracef("File ignored: %s / %s", args.Rel, f)
			continue
		}

		BazelLog.Tracef("GazelleWalkDir RegularFile: %s", f)

		walkErr := walkFunc(f)
		if walkErr != nil && walkErr != filepath.SkipDir {
			return walkErr
		}
	}

	// Do not manually traverse Subdirs unless specified
	if !recurse {
		return nil
	}

	// If the aspect "walksubdir" patch has been applied to gazelle then no manual
	// recursing into the subdirectories is required.
	if _, hasWalksubdirPatch := args.Config.Exts[ASPECT_WALKSUBDIR_PATCHED]; hasWalksubdirPatch {
		return nil
	}

	BazelLog.Warnf("WARNING: Aspect patches not applied, manual subdirectory traversal: %s", args.Rel)

	// Source files throughout the sub-directories of this BUILD.
	for _, d := range args.Subdirs {
		err := filepath.WalkDir(
			path.Join(args.Dir, d),
			func(filePath string, info os.DirEntry, err error) error {
				if err != nil {
					return err
				}

				// Skip BUILD files
				if args.Config.IsValidBuildFileName(path.Base(filePath)) {
					return nil
				}

				// The filePath relative to the BUILD
				f, _ := filepath.Rel(args.Dir, filePath)

				var excludeResult error = nil
				if info.IsDir() {
					excludeResult = filepath.SkipDir
				}

				// Gazelle-excluded paths. Must be done manually for subdirs.
				if IsFileExcluded(args.Rel, f, excludes) {
					BazelLog.Tracef("File excluded: %s / %s", args.Rel, f)
					return excludeResult
				} else if isIgnored(path.Join(args.Rel, f)) {
					// Ignored paths
					BazelLog.Tracef("File ignored: %s / %s", args.Rel, f)
					return excludeResult
				}

				// If visiting a directory recurse if it is not a bazel package.
				if info.IsDir() {
					if IsBazelPackage(args.Config, filePath) {
						return filepath.SkipDir
					}
					return nil
				}

				BazelLog.Tracef("GazelleWalkDir Subdir file: %s", f)

				return walkFunc(f)
			},
		)

		if err != nil && err != filepath.SkipDir {
			return err
		}
	}

	return nil
}
