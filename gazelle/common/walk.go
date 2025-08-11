package gazelle

import (
	"log"
	"slices"
	"strings"

	BazelLog "github.com/aspect-build/aspect-cli/gazelle/common/logger"
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

	// Navigate into subdirectories...
	// - Do not allocate arrays such as string.Split()
	// - Do not allow path.Join/Clean() multiple times on the same paths
	for i := strings.IndexByte(p, '/'); i >= 0; i = strings.IndexByte(p, '/') {
		subdir := p[:i]
		if !slices.Contains(d.Subdirs, subdir) {
			return false
		}
		if rel == "" {
			rel = subdir
		} else {
			rel = rel + "/" + subdir
		}
		d, err = walk.GetDirInfo(rel)
		if err != nil {
			log.Fatal(err)
		}

		p = p[i+1:]
	}

	return slices.Contains(d.RegularFiles, p)
}

func GetSourceRegularFiles(rel string) ([]string, error) {
	d, err := walk.GetDirInfo(rel)
	if err != nil {
		return nil, err
	}

	if rel != "" {
		rel = rel + "/"
	}
	return getSourceRegularSubFiles(rel, "", d, d.RegularFiles[:])
}

func getSourceRegularSubFiles(base, rel string, d walk.DirInfo, files []string) ([]string, error) {
	for _, sdRel := range d.Subdirs {
		if rel != "" {
			sdRel = rel + "/" + sdRel
		}

		sdInfo, _ := walk.GetDirInfo(base + sdRel)

		// Recurse into subdirectories that do not have a BUILD file just like a
		// bazel BUILD glob() would.
		if sdInfo.File == nil {
			for _, f := range sdInfo.RegularFiles {
				files = append(files, sdRel+"/"+f)
			}

			files, _ = getSourceRegularSubFiles(base, sdRel, sdInfo, files)
		}
	}

	return files, nil
}
