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

package license

import (
	"context"
	_ "embed"
	"fmt"

	"github.com/aspect-build/aspect-cli/buildinfo"
	"github.com/aspect-build/aspect-cli/pkg/bazel"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/spf13/cobra"
)

type License struct {
	ioutils.Streams
	bzl         bazel.Bazel
	licenseText string
}

//go:embed LICENSE
var defaultLicense string

func New(streams ioutils.Streams, bzl bazel.Bazel, licenseText string) *License {
	if len(licenseText) == 0 {
		licenseText = defaultLicense
	}

	return &License{
		Streams:     streams,
		bzl:         bzl,
		licenseText: licenseText,
	}
}

func (runner *License) Run(ctx context.Context, _ *cobra.Command, args []string) error {
	// ASCII art generated with https://patorjk.com/software/taag/ "ANSI Shadow" font
	if buildinfo.Current().OpenSource {
		fmt.Printf(`
=====================================================================================================

 █████╗ ███████╗██████╗ ███████╗ ██████╗████████╗     ██████╗██╗     ██╗     ██████╗ ███████╗███████╗
██╔══██╗██╔════╝██╔══██╗██╔════╝██╔════╝╚══██╔══╝    ██╔════╝██║     ██║    ██╔═══██╗██╔════╝██╔════╝
███████║███████╗██████╔╝█████╗  ██║        ██║       ██║     ██║     ██║    ██║   ██║███████╗███████╗
██╔══██║╚════██║██╔═══╝ ██╔══╝  ██║        ██║       ██║     ██║     ██║    ██║   ██║╚════██║╚════██║
██║  ██║███████║██║     ███████╗╚██████╗   ██║       ╚██████╗███████╗██║    ╚██████╔╝███████║███████║
╚═╝  ╚═╝╚══════╝╚═╝     ╚══════╝ ╚═════╝   ╚═╝        ╚═════╝╚══════╝╚═╝     ╚═════╝ ╚══════╝╚══════╝

=====================================================================================================		

`)
	} else {
		fmt.Printf(`
=====================================================================================================

               █████╗ ███████╗██████╗ ███████╗ ██████╗████████╗     ██████╗██╗     ██╗
              ██╔══██╗██╔════╝██╔══██╗██╔════╝██╔════╝╚══██╔══╝    ██╔════╝██║     ██║
              ███████║███████╗██████╔╝█████╗  ██║        ██║       ██║     ██║     ██║
              ██╔══██║╚════██║██╔═══╝ ██╔══╝  ██║        ██║       ██║     ██║     ██║
              ██║  ██║███████║██║     ███████╗╚██████╗   ██║       ╚██████╗███████╗██║
              ╚═╝  ╚═╝╚══════╝╚═╝     ╚══════╝ ╚═════╝   ╚═╝        ╚═════╝╚══════╝╚═╝

=====================================================================================================

`)
	}
	fmt.Print(runner.licenseText)

	// ASCII art generated with https://patorjk.com/software/taag/ "Standard" font
	fmt.Printf(`
=====================================================================================================
                    ____                _   _     _
                   | __ )  __ _ _______| | | |   (_) ___ ___ _ __  ___  ___ ___
                   |  _ \ / _' |_  / _ | | | |   | |/ __/ _ | '_ \/ __|/ _ / __|
                   | |_) | (_| |/ |  __| | | |___| | (_|  __| | | \__ |  __\__ \
                   |____/ \__,_/___\___|_| |_____|_|\___\___|_| |_|___/\___|___/

=====================================================================================================

`)

	bazelCmd := []string{"license"}
	bazelCmd = append(bazelCmd, args...)
	return runner.bzl.RunCommand(runner.Streams, nil, bazelCmd...)
}
