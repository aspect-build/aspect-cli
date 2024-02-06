package gazelle

import (
	"os"
	"path"
	"path/filepath"

	"aspect.build/cli/gazelle/common/git"
	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/language"
)

type GazelleWalkFunc func(path string) error

// Walk the directory of the language.GenerateArgs, optionally recursing into
// subdirectories unlike the files provided in GenerateArgs.RegularFiles.
func GazelleWalkDir(args language.GenerateArgs, ignore *git.GitIgnore, recurse bool, walkFunc GazelleWalkFunc) error {
	BazelLog.Tracef("GazelleWalkDir: %s", args.Rel)

	// Source files in the primary directory
	for _, f := range args.RegularFiles {
		// Skip BUILD files
		if isBuildFile(f) {
			continue
		}

		if ignore.Matches(path.Join(args.Rel, f)) {
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

	// Source files throughout the sub-directories of this BUILD.
	for _, d := range args.Subdirs {
		err := filepath.WalkDir(
			path.Join(args.Dir, d),
			func(filePath string, info os.DirEntry, err error) error {
				if err != nil {
					return err
				}

				// If we are visiting a directory recurse if it is not a bazel package.
				if info.IsDir() {
					if IsBazelPackage(filePath) {
						return filepath.SkipDir
					}
					return nil
				}

				// Skip BUILD files
				if isBuildFile(filePath) {
					return nil
				}

				// The filePath relative to the BUILD
				f, _ := filepath.Rel(args.Dir, filePath)

				if ignore.Matches(path.Join(args.Rel, f)) {
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
