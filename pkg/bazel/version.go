/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package bazel

import (
	"bufio"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"aspect.build/cli/buildinfo"
)

const (
	aspectBuildVersionPrefix    = "aspect-build/"
	defaultBazelVersionBasename = ".bazelversion"
)

func VersionPath(workspaceRoot string) string {
	return filepath.Join(workspaceRoot, defaultBazelVersionBasename)
}

type Version struct {
	Bazel  string
	Aspect string
}

// Returns a version with default values.
func NewVersion() *Version {
	bi := buildinfo.Current()
	return &Version{
		Bazel:  "",
		Aspect: bi.Release,
	}
}

func NewVersionFromReader(reader io.Reader) (*Version, error) {
	bVer := ""
	aVer := ""

	scanner := bufio.NewScanner(reader)
	scanner.Scan()
	for scanner.Scan() {
		t := strings.TrimSpace(scanner.Text())
		if strings.HasPrefix(t, aspectBuildVersionPrefix) {
			if aVer != "" {
				return nil, fmt.Errorf("encountered multiple `aspect-build` version declarations")
			}
			prefixRunes := []rune(aspectBuildVersionPrefix)
			runes := []rune(t)
			aVer = string(runes[len(prefixRunes):])
		} else if bVer == "" {
			bVer = t
		} else {
			return nil, fmt.Errorf("unexpected line while reading Bazel version. line: %s", t)
		}
	}

	// If the file did not provide any version values, return a default version
	if bVer == "" && aVer == "" {
		return NewVersion(), nil
	}

	return &Version{
		Bazel:  bVer,
		Aspect: aVer,
	}, nil
}

func NewVersionFromFile(path string) (*Version, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	return NewVersionFromReader(f)
}

func SafeVersionFromFile(path string) (*Version, error) {
	if _, err := os.Stat(path); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return NewVersion(), nil
		}
		return nil, err
	}
	return NewVersionFromFile(path)
}

// func (v Version) InitAspect() {
// }

// func (v Version) Write(path string) error {
// }

