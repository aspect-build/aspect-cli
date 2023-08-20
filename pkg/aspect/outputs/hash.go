package outputs

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"strings"

	"aspect.build/cli/pkg/bazel"
	"golang.org/x/mod/sumdb/dirhash"
)

// AddExecutableHash appends the exePath to hashFiles entry of the label
func AddExecutableHash(hashFiles map[string][]string, label string, exePath string) {
	_, err := os.Stat(exePath)
	if os.IsNotExist(err) {
		fmt.Fprintf(os.Stderr, "%s output %s is not on disk, did you build it? Skipping...\n", label, exePath)
		return
	}

	hashFiles[label] = append(hashFiles[label], exePath)
}

// AddRunfilesHash iterates through the runfiles entries of the manifest, appending all files
// contained (or files inside directories) to the hashFiles entry of the label
func AddRunfilesHash(hashFiles map[string][]string, label string, manifestPath string) error {
	_, err := os.Stat(manifestPath)
	if os.IsNotExist(err) {
		fmt.Fprintf(os.Stderr, "%s manifest %s is not on disk, did you build it? Skipping...\n", label, manifestPath)
		return nil
	}
	runfiles, err := os.Open(manifestPath)
	if err != nil {
		return fmt.Errorf("failed to open runfiles manifest %s: %w\n", manifestPath, err)
	}
	defer runfiles.Close()

	fileScanner := bufio.NewScanner(runfiles)
	fileScanner.Split(bufio.ScanLines)

	for fileScanner.Scan() {
		// Manifest entries are in the form
		// execroot/path /some/absolute/path
		entry := strings.Split(fileScanner.Text(), " ")
		// key := entry[0]
		abspath := entry[1]
		fileinfo, err := os.Stat(abspath)

		if err != nil {
			return fmt.Errorf("failed to stat runfiles manifest entry %s: %w\n", abspath, err)
		}

		if fileinfo.IsDir() {
			// TODO(alexeagle): I think the abspath means we'll get more hashed than we mean to
			// we should pass some other value to the second arg "prefix"
			direntries, err := dirhash.DirFiles(abspath, abspath)
			if err != nil {
				return fmt.Errorf("failed to recursively list directory %s: %w\n", abspath, err)
			}
			hashFiles[label] = append(hashFiles[label], direntries...)
		} else {
			hashFiles[label] = append(hashFiles[label], abspath)
		}
	}
	return nil
}

func gatherExecutableHashes(outs []bazel.Output) (map[string]string, error) {
	// map from Label to the files/directories which should be hashed
	hashFiles := make(map[string][]string)

	for _, a := range outs {
		if a.Mnemonic == "ExecutableSymlink" {
			AddExecutableHash(hashFiles, a.Label, a.Path)
		} else if a.Mnemonic == "SourceSymlinkManifest" {
			if err := AddRunfilesHash(hashFiles, a.Label, a.Path); err != nil {
				return nil, err
			}
		}
	}

	osOpen := func(name string) (io.ReadCloser, error) {
		return os.Open(name)
	}
	result := make(map[string]string)
	for label, files := range hashFiles {
		overallhash, err := dirhash.Hash1(files, osOpen)
		if err != nil {
			return nil, fmt.Errorf("failed to compute runfiles hash for manifest: %w\n", err)
		}
		result[label] = overallhash
	}
	return result, nil
}
