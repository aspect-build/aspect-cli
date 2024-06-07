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

	"aspect.build/cli/buildinfo"

	"github.com/spf13/cobra"
)

func AspectLockVersionDefault() bool {
	// We set the default `--aspect:lock_version`` depending on whether or not the CLI is a stamped
	// release or unstamped development build. When unstamped we want to ignore the version of the
	// CLI specified in `.bazeliskrc` or `.aspect/cli/config.yaml` and not re-enter that version
	// when booting since during development that is more ergonomic. For stamped release builds, on
	// the other hand, we want to CLI to honor the version specified in the repos `.bazeliskrc` or
	// `.aspect/cli/config.yaml` since that is the version that developers are expecting to be run.
	// In either case, the default behavior can be overridden by explicitly specifying the flag as
	// either `--aspect:lock_version` or `--aspect:lock_version=false`.
	return !buildinfo.Current().HasRelease()
}

func AddGlobalFlags(cmd *cobra.Command, defaultInteractive bool) {
	// Documented global flags. These flags show up as "global flags" on the `help` command output.
	cmd.PersistentFlags().String(AspectConfigFlagName, "", fmt.Sprintf("User-specified Aspect CLI config file. /dev/null indicates that all further --%s flags will be ignored.", AspectConfigFlagName))
	cmd.PersistentFlags().Bool(AspectInteractiveFlagName, defaultInteractive, "Interactive mode (e.g. prompts for user input)")

	// Hidden global flags
	cmd.PersistentFlags().Bool(AspectLockVersion, AspectLockVersionDefault(), "Lock the version of the Aspect CLI. This prevents the Aspect CLI from downloading and running an different version of the Aspect CLI if one is specified in .bazeliskrc or the Aspect CLI config.")
	cmd.PersistentFlags().MarkHidden(AspectLockVersion)

	cmd.PersistentFlags().Bool(AspectForceBesBackendFlagName, false, "Force the creation of a BES backend even if there are no plugins loaded")
	cmd.PersistentFlags().MarkHidden(AspectForceBesBackendFlagName)

	cmd.PersistentFlags().Bool(AspectDisablePluginsFlagName, false, "Disable the plugin system. This prevents Aspect CLI for starting any plugins.")
	cmd.PersistentFlags().MarkHidden(AspectDisablePluginsFlagName)

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
