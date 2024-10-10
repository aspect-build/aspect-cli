package gazelle

import (
	"log"
	"strings"

	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/config"
	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/bazelbuild/bazel-gazelle/rule"
)

type GazelleWalkFunc func(path string) error

// Must align with patched bazel-gazelle
const ASPECT_WALKSUBDIR = "__aspect:walksubdir"

// Read any configuration regarding walk options.
func ReadWalkConfig(c *config.Config, rel string, f *rule.File) bool {
	if f == nil {
		v, ok := c.Exts[ASPECT_WALKSUBDIR]
		return !ok || !v.(bool)
	}

	for _, d := range f.Directives {
		switch d.Key {
		case Directive_GenerationMode:
			switch GenerationModeType(strings.TrimSpace(d.Value)) {
			case GenerationModeCreate:
				c.Exts[ASPECT_WALKSUBDIR] = false
			case GenerationModeUpdate:
				c.Exts[ASPECT_WALKSUBDIR] = true
			default:
				log.Fatalf("invalid value for directive %q: %s", Directive_GenerationMode, d.Value)
			}
		}
	}
	return true
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
