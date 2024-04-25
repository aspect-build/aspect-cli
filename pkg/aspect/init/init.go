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

package init

import (
	"context"
	"embed"
	"fmt"
	"io/fs"
	"os"
	"path"
	"strings"

	"aspect.build/cli/buildinfo"
	"aspect.build/cli/docs/bazelrc"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/bazel/workspace"
	"aspect.build/cli/pkg/ioutils"

	semver "github.com/Masterminds/semver/v3"
	"github.com/fatih/color"
	"github.com/manifoldco/promptui"
	"github.com/spf13/cobra"
)

//go:embed template/** template/**/_*
var Content embed.FS

var (
	boldCyan = color.New(color.FgCyan, color.Bold)
)

type Init struct {
	ioutils.Streams
}

func New(streams ioutils.Streams) *Init {
	return &Init{
		Streams: streams,
	}
}

func (runner *Init) lookupAspectVersion() (string, error) {
	aspectVersions, err := bazel.GetAspectVersions()
	if err == nil && len(aspectVersions) > 0 {
		return aspectVersions[0], nil
	}

	bi := *buildinfo.Current()

	if !bi.HasRelease() {
		return "", fmt.Errorf("Could not determine latest aspect release and current version is unstamped: %w", err)
	}

	// if we fail to get the latest release of Aspect CLI then fallback to stamping the current version
	versionWithMeta, err := semver.NewVersion(bi.Release)
	if err != nil {
		return "", fmt.Errorf("Could not determine latest aspect release and failed to parse current version '%s' semver: %w", bi.Release, err)
	}

	// throw away metadata
	version, err := versionWithMeta.SetMetadata("")
	if err != nil {
		return "", fmt.Errorf("Could not determine latest aspect release and failed to parse current version '%s' semver: %w", bi.Release, err)
	}

	if version.Prerelease() != "" {
		if strings.HasPrefix(version.Prerelease(), "dev.") {
			// special case for dev version; bump the patch down to determine the last release
			if version.Patch() == 0 {
				// patch should never be 0 on a dev version
				return "", fmt.Errorf("Could not determine latest aspect release and failed to parse current version '%s' semver: %w", bi.Release, err)
			}
			return fmt.Sprintf("%v.%v.%v", version.Major(), version.Minor(), version.Patch()-1), nil
		}
	}

	return version.String(), nil
}

func (runner *Init) Run(ctx context.Context, _ *cobra.Command, args []string) error {
	wd, err := os.Getwd()
	if err != nil {
		return err
	}
	finder := workspace.DefaultFinder
	wr, err := finder.Find(wd)

	if err == nil {
		fmt.Printf("The current working directory is already inside a Bazel workspace rooted at %s.\n", wr)
		fmt.Println("Skipping new workspace creation...")
		// TODO: mention 'doctor' command when we have it
		// TODO: offer to add more stuff to the existing workspace, like language-specific support
		return nil
	}

	// figure out what version of Aspect to stamp out
	aspectVersion, err := runner.lookupAspectVersion()
	if err != nil {
		return err
	}

	if len(args) > 0 {
		return initNewWorkspace(args[0], aspectVersion)
	}

	prompt := promptui.Select{
		Label: "Where would you like to create a Bazel workspace",
		Items: []string{
			fmt.Sprintf("Current directory (%s)", wd),
			"Create a new directory under " + wd,
		},
	}

	choice, _, err := prompt.Run()

	if err != nil {
		return fmt.Errorf("user aborted the wizard: %w", err)
	}

	if choice == 0 {
		return initNewWorkspace(".", aspectVersion)
	}
	if choice == 1 {
		prompt := promptui.Prompt{
			Label: "Name for the new folder",
		}

		folder, err := prompt.Run()

		if err != nil {
			return fmt.Errorf("user aborted the wizard: %w", err)
		}
		return initNewWorkspace(folder, aspectVersion)
	}
	return fmt.Errorf("illegal state: no choices matched, please file a bug")
}

func initNewWorkspace(folder string, aspectVersion string) error {
	var cdmsg string
	if folder != "." {
		fmt.Printf("Creating folder %s...\n", folder)
		if err := os.Mkdir(folder, 0755); err != nil {
			return fmt.Errorf("failed to create directory %s: %w", folder, err)
		}
		os.Chdir(folder)
		cdmsg = fmt.Sprintf("run 'cd %s', then ", folder)
	}
	if err := os.MkdirAll(path.Join(".aspect", "bazelrc"), 0755); err != nil {
		return fmt.Errorf("failed to create directory: %w", err)
	}

	err := fs.WalkDir(Content, ".", func(tmpl string, f fs.DirEntry, err error) error {
		if err != nil {
			return fmt.Errorf("unable to list embedded files: %w", err)
		}
		out := strings.TrimPrefix(tmpl, "template/")
		if f.IsDir() {
			os.Mkdir(out, 0755)
		} else {
			content, err := Content.ReadFile(tmpl)
			if err != nil {
				return fmt.Errorf("unable to read template file %s: %w", tmpl, err)
			}
			if f.Name() == ".bazeliskrc" {
				// replace the {{aspect_version}} token in the bazeliskrc template to the desired aspect version
				// TODO: use https://pkg.go.dev/text/template instead
				content = []byte(strings.Replace(string(content), "{{aspect_version}}", aspectVersion, 1))
			}
			var mode fs.FileMode
			if strings.HasSuffix(out, ".sh") {
				mode = 0755
			} else {
				mode = 0644
			}
			if err = os.WriteFile(out, content, mode); err != nil {
				return fmt.Errorf("failed to write file: %w", err)
			} else {
				fmt.Printf("wrote %s\n", out)
			}
		}
		return nil
	})
	if err != nil {
		return fmt.Errorf("unable to walk embed folder: %w", err)
	}

	rcs, err := bazelrc.Content.ReadDir(".")
	if err != nil {
		return fmt.Errorf("unable to list embed files: %w", err)
	}

	for _, p := range rcs {
		f := path.Join(".aspect", "bazelrc", p.Name())
		content, err := bazelrc.Content.ReadFile(p.Name())
		if err != nil {
			return fmt.Errorf("failed to read embedded data: %w", err)
		}
		if err = os.WriteFile(f, content, 0644); err != nil {
			return fmt.Errorf("failed to write file: %w", err)
		} else {
			fmt.Printf("wrote %s\n", f)
		}
	}

	boldCyan.Println("Finished creating Bazel workspace.")
	fmt.Printf("To confirm this is working, %srun 'bazel fetch //...'\n", cdmsg)
	fmt.Println("Next step: create BUILD.bazel files. Consider 'aspect configure' to automate this.")
	return nil
}
