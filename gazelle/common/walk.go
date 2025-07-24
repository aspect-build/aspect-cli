package gazelle

import (
	"log"
	"path"
	"slices"
	"strings"

	BazelLog "github.com/aspect-build/aspect-cli/pkg/logger"
	"github.com/bazelbuild/bazel-gazelle/language"
	"github.com/bazelbuild/bazel-gazelle/walk"
)

type GazelleWalkFunc func(path string) error

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

func WalkHasPath(rel, p string) bool {
	d, err := walk.GetDirInfo(rel)
	if err != nil {
		log.Fatal(err)
	}

	base := path.Base(p)

	// Navigate into subdirs
	if subdir := path.Dir(p); subdir != "." {
		for _, pp := range strings.Split(subdir, "/") {
			if !slices.Contains(d.Subdirs, pp) {
				return false
			}
			rel = path.Join(rel, pp)
			d, err = walk.GetDirInfo(rel)
			if err != nil {
				log.Fatal(err)
			}
		}
	}

	return slices.Contains(d.RegularFiles, base)
}

func GetSourceRegularFiles(rel string) ([]string, error) {
	d, err := walk.GetDirInfo(rel)
	if err != nil {
		return nil, err
	}

	return getSourceRegularSubFiles(rel, ".", d, d.RegularFiles[:])
}

func getSourceRegularSubFiles(base, rel string, d walk.DirInfo, files []string) ([]string, error) {
	for _, sd := range d.Subdirs {
		sdRel := path.Join(rel, sd)
		sdInfo, _ := walk.GetDirInfo(path.Join(base, sdRel))

		for _, f := range sdInfo.RegularFiles {
			files = append(files, path.Join(sdRel, f))
		}

		files, _ = getSourceRegularSubFiles(base, sdRel, sdInfo, files)
	}

	return files, nil
}
