package gazelle

import (
	"maps"
	"slices"

	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/language"
)

type GazelleWalkFunc func(path string) error

// Must align with patched bazel-gazelle
const ASPECT_DIR_FILES = "__aspect:files"

func GetSourceEntries(c *config.Config) map[string]bool {
	return c.Exts[ASPECT_DIR_FILES].(map[string]bool)
}

func GetSourceRegularFiles(c *config.Config) []string {
	return slices.Collect(maps.Keys(GetSourceEntries(c)))
}

// Walk the directory being generated, respecting any walk generation config.
func GazelleWalkDir(args language.GenerateArgs, walkFunc GazelleWalkFunc) error {
	BazelLog.Tracef("GazelleWalkDir: %s", args.Rel)

	// Source files in the primary directory
	for _, f := range args.RegularFiles {
		// Skip BUILD files
		if args.Config.IsValidBuildFileName(f) {
			continue
		}

		if walkErr := walkFunc(f); walkErr != nil {
			return walkErr
		}
	}

	return nil
}
