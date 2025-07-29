package bazel

import (
	"bufio"
	"errors"
	"fmt"
	"io/fs"
	"log"
	"os"
	"path"
	"strings"
	"sync"
)

var ignores sync.Map

func LoadBazelIgnore(repoRoot string) ([]string, error) {
	if v, ok := ignores.Load(repoRoot); ok {
		return v.([]string), nil
	}

	loaded, err := loadBazelIgnore(repoRoot)
	if err != nil {
		return nil, err
	}

	excludes, _ := ignores.LoadOrStore(repoRoot, loaded)
	return excludes.([]string), nil
}

func loadBazelIgnore(repoRoot string) ([]string, error) {
	ignorePath := path.Join(repoRoot, ".bazelignore")
	file, err := os.Open(ignorePath)
	if errors.Is(err, fs.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, fmt.Errorf(".bazelignore exists but couldn't be read: %v", err)
	}
	defer file.Close()

	excludes := []string{}

	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		ignore := strings.TrimSpace(scanner.Text())
		if ignore == "" || string(ignore[0]) == "#" {
			continue
		}
		// Bazel ignore paths are always relative to repo root.
		// Glob patterns are not supported.
		if strings.ContainsAny(ignore, "*?[") {
			log.Printf("the .bazelignore exclusion pattern must not be a glob %s", ignore)
			continue
		}

		// Clean the path to remove any extra '.', './' etc otherwise
		// the exclude matching won't work correctly.
		ignore = path.Clean(ignore)

		excludes = append(excludes, ignore)
	}

	return excludes, nil
}
