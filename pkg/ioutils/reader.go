package ioutils

import (
	"aspect.build/cli/pkg/pathutils"
	"bufio"
	"os"
	"strings"
)

var stdInReader = bufio.NewReader(os.Stdin)

func ReadLine() (string, error) {
	path, err := stdInReader.ReadString('\n')
	if err != nil {
		return "", err
	}
	// convert CRLF to LF for Windows compatibility
	return strings.Replace(path, "\n", "", -1), nil
}

// ReadCommonTargets returns the targets specified underneath the aspect:default comment for a
// specified path to a package root as a space separated string
func ReadCommonTargets(pkgPath string) ([]string, error) {
	pkgPath = strings.TrimRight(pkgPath, "/")
	buildFilePath, err := pathutils.GetBuildFilePath(pkgPath)
	if err != nil {
		return []string{}, err
	}
	buildFile, err := os.Open(buildFilePath)
	if err != nil {
		return []string{}, err
	}
	defer buildFile.Close()
	fileReader := bufio.NewScanner(buildFile)
	targets := []string{}
	targetsSpecified := false
	for fileReader.Scan() {
		line := fileReader.Text()
		if targetsSpecified {
			if strings.HasPrefix(line, "#") {
				target := strings.TrimSpace(strings.TrimPrefix(line, "#"))
				// TODO: refactor into function to validate target label
				if strings.HasPrefix(target, "//") {
					targets = append(targets, target)
				} else if strings.HasPrefix(target, ":") {
					targets = append(targets, "//" + pkgPath + target)
				}
			} else {
				targetsSpecified = false
			}
		}
		if strings.HasPrefix(line, "# aspect:default") {
			targetsSpecified = true
		}
	}
	if err := fileReader.Err(); err != nil {
		return []string{}, err
	}
	return targets, nil
}