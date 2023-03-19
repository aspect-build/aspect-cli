package gazelle

import (
	"os"
	"path"
	"path/filepath"

	"github.com/bazelbuild/bazel-gazelle/language"
)

// Walk the directory of the language.GenerateArgs, optionally recursing into
// subdirectories unlike the files provided in GenerateArgs.RegularFiles.
func GazelleWalkDir(args language.GenerateArgs, recurse bool, walkFunc filepath.WalkFunc) error {
	BazelLog.Tracef("GazelleWalkDir: %s", args.Rel)

	// Stat the directory
	rootInfo, rootInfoErr := os.Stat(args.Dir)
	if rootInfoErr != nil {
		return rootInfoErr
	}

	// Callback on the directory itself
	rootWalkErr := walkFunc(".", rootInfo, nil)
	if rootWalkErr != nil {
		return rootWalkErr
	}

	// Source files in the primary directory
	for _, f := range args.RegularFiles {
		BazelLog.Tracef("GazelleWalkDir RegularFile: %s", f)

		info, infoErr := os.Stat(filepath.Join(args.Dir, f))
		if infoErr != nil {
			return infoErr
		}

		walkErr := walkFunc(f, info, infoErr)
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
		err := filepath.Walk(
			path.Join(args.Dir, d),
			func(filePath string, info os.FileInfo, err error) error {
				if err != nil {
					return err
				}

				// If we are visiting a directory recurse if it is not a bazel package.
				if info.IsDir() && isBazelPackage(filePath) {
					return filepath.SkipDir
				}

				// Skip BUILD files
				if isBuildFile(filePath) {
					return nil
				}

				// The filePath relative to the BUILD
				f, _ := filepath.Rel(args.Dir, filePath)

				BazelLog.Tracef("GazelleWalkDir Subdir file: %s", f)

				return walkFunc(f, info, nil)
			},
		)

		if err != nil && err != filepath.SkipDir {
			return err
		}
	}

	return nil
}
