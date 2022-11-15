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

package pro

import (
	"context"
	"fmt"

	"aspect.build/cli/pkg/aspect/root/config"
	"aspect.build/cli/pkg/ioutils"
	"github.com/manifoldco/promptui"
	"github.com/spf13/cobra"
)

type Pro struct {
	ioutils.Streams
}

func New(streams ioutils.Streams) *Pro {
	return &Pro{
		Streams: streams,
	}
}

func (v *Pro) Run(ctx context.Context, _ *cobra.Command, args []string) error {
	if err := enableProForUser(); err != nil {
		return err
	}
	if err := enableProForWorkspace(); err != nil {
		return err
	}
	return nil
}

// TODO: move promptui helpers to shared library
type ConfirmationRunner interface {
	Run() (string, error)
}

func Confirmation(question string) ConfirmationRunner {
	return &promptui.Prompt{
		Label:     question,
		IsConfirm: true,
	}
}

func enableProForUser() error {
	homeConfig, err := config.LoadHomeConfig()
	if err != nil {
		return err
	}

	version, err := config.ParseConfigVersion(homeConfig.GetString("version"))
	proTier := config.IsProTier(version.Tier)
	if !proTier {
		_, err = Confirmation("Enable Aspect CLI Pro features for user").Run()
		if err == nil {
			// TODO: show Aspect commercial license when it is finalized and/or send user to a "Sign up for free trial period" page
			configFile, created, err := config.SetInHomeConfig("version", toProVersion(version.Version))
			if err != nil {
				return err
			}
			if created {
				fmt.Printf("Created %s\n", configFile)
			} else {
				fmt.Printf("Updated %s\n", configFile)
			}
		}
	} else {
		configFile, _ := config.HomeConfigFile()
		if configFile == "" {
			fmt.Println("Aspect CLI Pro features already enabled for user")
		} else {
			fmt.Printf("Aspect CLI Pro features already enabled for user in %s\n", configFile)
		}
	}

	return nil
}

func enableProForWorkspace() error {
	workspaceConfig, err := config.LoadWorkspaceConfig()
	if err != nil {
		return err
	}

	version, err := config.ParseConfigVersion(workspaceConfig.GetString("version"))
	proTier := config.IsProTier(version.Tier)
	if !proTier {
		_, err = Confirmation("Enable Aspect CLI Pro features for workspace").Run()
		if err == nil {
			// TODO: show Aspect commercial license when it is finalized and/or send user to a "Sign up for free trial period" page
			configFile, created, err := config.SetInWorkspaceConfig("version", toProVersion(version.Version))
			if err != nil {
				return err
			}
			if created {
				fmt.Printf("Created %s\n", configFile)
			} else {
				fmt.Printf("Updated %s\n", configFile)
			}
		}
	} else {
		configFile, _ := config.WorkspaceConfigFile()
		if configFile == "" {
			fmt.Println("Aspect CLI Pro features already enabled for workspace")
		} else {
			fmt.Printf("Aspect CLI Pro features already enabled for workspace in %s\n", configFile)
		}
	}

	return nil
}

func toProVersion(version string) string {
	if len(version) == 0 {
		return "pro"
	} else {
		return fmt.Sprintf("pro/%s", version)
	}
}
