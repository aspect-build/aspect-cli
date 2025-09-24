/*
 * Copyright 2023 Aspect Build Systems, Inc.
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

/*
 * Copyright 2024 Aspect Build Systems, Inc.
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

package configure

import (
	"fmt"
	"os"
	"path"
	"strings"

	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	BazelLog "github.com/aspect-build/orion/common/logger"
)

// The @gazelle go_deps extension name.
// https://github.com/bazel-contrib/bazel-gazelle/blob/v0.39.1/internal/bzlmod/go_deps.bzl#L827
const GO_DEPS_EXTENSION_NAME = "go_deps"

// The repository name for the gazelle repo_config.
// https://github.com/bazel-contrib/bazel-gazelle/blob/v0.39.1/internal/bzlmod/go_deps.bzl#L648-L654
const GO_REPOSITORY_CONFIG_REPO_NAME = "bazel_gazelle_go_repository_config"

// bazel 8 switches the bzlmod separator to "+"
// See https://github.com/bazelbuild/bazel/issues/23127
var BZLMOD_REPO_SEPARATORS = []string{"~", "+"}

func determineGoRepositoryConfigPath() (string, error) {
	// TODO(jason): look into a store of previous invocations for relevant logs
	bzl := bazel.WorkspaceFromWd

	var out strings.Builder
	streams := ioutils.Streams{Stdout: &out, Stderr: nil}
	if err := bzl.RunCommand(streams, nil, "info", "output_base"); err != nil {
		return "", fmt.Errorf("unable to locate output_base: %w", err)
	}

	outputBase := strings.TrimSpace(out.String())
	if outputBase == "" {
		return "", fmt.Errorf("unable to locate output_base on path")
	}

	var goDepsRepoName string
	for _, sep := range BZLMOD_REPO_SEPARATORS {
		repoName := fmt.Sprintf("gazelle%s%s%s%s%s/WORKSPACE", sep, sep, GO_DEPS_EXTENSION_NAME, sep, GO_REPOSITORY_CONFIG_REPO_NAME)
		repoPath := path.Join(outputBase, "external", repoName)

		_, err := os.Stat(repoPath)
		if err == nil {
			goDepsRepoName = repoPath
			break
		}
	}

	if goDepsRepoName == "" {
		// Assume no matches means rules_go is not being used in bzlmod
		// or the gazelle `go_deps` extension is not being used
		BazelLog.Infof("No %s found in output_base: %s", GO_REPOSITORY_CONFIG_REPO_NAME, outputBase)
		return "", nil
	}

	BazelLog.Infof("Found %s(s): %v", GO_REPOSITORY_CONFIG_REPO_NAME, goDepsRepoName)

	return goDepsRepoName, nil
}
