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

package flags

import (
	"fmt"

	"github.com/spf13/cobra"
)

func AddGlobalFlags(cmd *cobra.Command, defaultInteractive bool) {
	// Documented global flags. These flags show up as "global flags" on the `help` command output.
	cmd.PersistentFlags().String(AspectConfigFlagName, "", fmt.Sprintf("User-specified Aspect CLI config file. /dev/null indicates that all further --%s flags will be ignored.", AspectConfigFlagName))
	cmd.PersistentFlags().Bool(AspectInteractiveFlagName, defaultInteractive, "Interactive mode (e.g. prompts for user input)")

	// Hidden global flags
	cmd.PersistentFlags().Bool(AspectLockVersion, false, "Lock the version of the Aspect CLI. This prevents the Aspect CLI from downloading and running an different version of the Aspect CLI if one is specified in .bazeliskrc or the Aspect CLI config.")
	cmd.PersistentFlags().MarkHidden(AspectLockVersion)

	RegisterNoableBool(cmd.PersistentFlags(), AspectSystemConfigFlagName, true, "Whether or not to look for the system config file at /etc/aspect/cli/config.yaml")
	cmd.PersistentFlags().MarkHidden(AspectSystemConfigFlagName)
	cmd.PersistentFlags().MarkHidden(NoFlagName(AspectSystemConfigFlagName))

	RegisterNoableBool(cmd.PersistentFlags(), AspectWorkspaceConfigFlagName, true, "Whether or not to look for the workspace config file at $workspace/.aspect/cli/config.yaml")
	cmd.PersistentFlags().MarkHidden(AspectWorkspaceConfigFlagName)
	cmd.PersistentFlags().MarkHidden(NoFlagName(AspectWorkspaceConfigFlagName))

	RegisterNoableBool(cmd.PersistentFlags(), AspectHomeConfigFlagName, true, "Whether or not to look for the home config file at $HOME/.aspect/cli/config.yaml")
	cmd.PersistentFlags().MarkHidden(AspectHomeConfigFlagName)
	cmd.PersistentFlags().MarkHidden(NoFlagName(AspectHomeConfigFlagName))
}
