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
	"fmt"
	"os"
	"path"
	"strings"

	"aspect.build/cli/docs/bazelrc"
	"aspect.build/cli/pkg/aspect/init/template"
	"aspect.build/cli/pkg/bazel/workspace"
	"aspect.build/cli/pkg/ioutils"
	"github.com/fatih/color"
	"github.com/manifoldco/promptui"
	"github.com/spf13/cobra"
)

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

	if len(args) > 0 {
		return initNewWorkspace(args[0])
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
		return initNewWorkspace(".")
	}
	if choice == 1 {
		prompt := promptui.Prompt{
			Label: "Name for the new folder",
		}

		folder, err := prompt.Run()

		if err != nil {
			return fmt.Errorf("user aborted the wizard: %w", err)
		}
		return initNewWorkspace(folder)
	}
	return fmt.Errorf("illegal state: no choices matched, please file a bug")
}

func initNewWorkspace(folder string) error {
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

	tmpls, err := template.Content.ReadDir(".")
	if err != nil {
		return fmt.Errorf("unable to list embed files: %w", err)
	}
	for _, f := range tmpls {
		content, err := template.Content.ReadFile(f.Name())
		if err != nil {
			return fmt.Errorf("unable to read embed file %s: %w", f.Name(), err)
		}
		out := strings.TrimSuffix(f.Name(), "_")
		if err = os.WriteFile(out, content, 0644); err != nil {
			return fmt.Errorf("failed to write file: %w", err)
		} else {
			fmt.Printf("wrote %s\n", out)
		}
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
