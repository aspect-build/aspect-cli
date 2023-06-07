package gazelle

import (
	"path"

	"github.com/bazelbuild/bazel-gazelle/language"
)

// Return the default target name for the given language.GenerateArgs.
// The default target name of a BUILD is the directory name. WHen within the repository
// root which may be outside of version control the default target name is the repository name.
func ToDefaultTargetName(args language.GenerateArgs, defaultRootName string) string {
	// The workspace root may be the version control root and non-deterministic
	if args.Rel == "" {
		if args.Config.RepoName != "" {
			return args.Config.RepoName
		} else {
			return defaultRootName
		}
	}

	return path.Base(args.Dir)
}
