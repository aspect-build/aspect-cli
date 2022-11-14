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
	"fmt"
	"strings"

	"aspect.build/cli/buildinfo"
	"github.com/spf13/viper"
)

type aspectConfig struct {
	ProTier bool
	Version string
	BaseUrl string
}

var aspectBaseUrls = map[bool]string{
	false: "https://github.com/aspect-build/aspect-cli/releases/download",
	true:  "https://static.aspect.build/aspect",
}

func getAspectConfig() (*aspectConfig, error) {
	configVersion := viper.GetString("version")
	configBaseUrl := viper.GetString("base_url")

	proTiers := map[string]bool{
		"":          false,
		"community": true,
		"pro":       true,
	}

	buildinfo := buildinfo.Current()

	// If there is a tier and/or version configured in the Aspect CLI config this takes precedence over a .bazeliskrc bootstrap version
	if configVersion != "" {
		tier := ""
		version := ""

		if configVersion[0] >= '0' && configVersion[0] <= '9' {
			// Only the version is specified; tier is derived above from running version
			version = configVersion
		} else {
			// The is a tier component to the version: <tier> or <tier>/1.2.3
			splits := strings.Split(configVersion, "/")
			if len(splits) == 1 {
				tier = splits[0]
				// Only tier is specified;  version is derived above from running version
			} else if len(splits) == 2 {
				// Both tier and version are specified in the Aspect CLI config
				tier = splits[0]
				version = splits[1]
			} else {
				return nil, fmt.Errorf("Aspect CLI version format in configuration: '%s'. Please fix your Aspect CLI configuration and try again.", configVersion)
			}
		}

		if version == "" {
			version = buildinfo.Version()
		}
		if tier == "" && buildinfo.IsAspectPro {
			tier = "pro"
		}

		proTier, tierOk := proTiers[tier]
		if !tierOk {
			return nil, fmt.Errorf("Unrecognized Aspect CLI tier in configuration: '%s'. Please fix your Aspect CLI configuration and try again.", tier)
		}

		if len(configBaseUrl) == 0 {
			configBaseUrl = aspectBaseUrls[proTier]
		}

		return &aspectConfig{
			ProTier: proTier,
			Version: version,
			BaseUrl: configBaseUrl,
		}, nil
	}

	if len(configBaseUrl) == 0 {
		configBaseUrl = aspectBaseUrls[false]
	}

	return &aspectConfig{
		ProTier: false,
		Version: "",
		BaseUrl: configBaseUrl,
	}, nil
}
