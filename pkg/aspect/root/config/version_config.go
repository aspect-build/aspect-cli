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

package config

import (
	"fmt"
	"strings"

	"aspect.build/cli/buildinfo"
	"github.com/spf13/viper"
	"golang.org/x/exp/maps"
)

var aspectProTiers = map[string]struct{}{
	"community": {},
	"pro":       {},
}

var validTiersCommaList = strings.Join(maps.Keys(aspectProTiers), ", ")

func IsValidTier(tier string) bool {
	if len(tier) == 0 {
		return true
	}
	_, ok := aspectProTiers[tier]
	return ok
}

func IsProTier(tier string) bool {
	if len(tier) == 0 {
		return false
	}
	_, ok := aspectProTiers[tier]
	return ok
}

func AspectBaseUrl(isProTier bool) string {
	if isProTier {
		return "https://static.aspect.build/aspect"
	} else {
		return "https://github.com/aspect-build/aspect-cli/releases/download"
	}
}

type VersionConfig struct {
	ProTier bool
	Version string
	BaseUrl string
}

func GetVersionConfig() (*VersionConfig, error) {
	tier, version, err := ParseConfigVersion(viper.GetString("version"))
	if err != nil {
		return nil, err
	}

	isProTier := false
	if tier == "" {
		if buildinfo.Current().IsAspectPro {
			tier = "pro"
			isProTier = true
		}
	} else {
		if !IsValidTier(tier) {
			return nil, fmt.Errorf("Unrecognized Aspect CLI tier in version in configuration: '%s'. Version should be [<tier>/]<version> with an optional tier set to one of: %s. Please fix your Aspect CLI configuration and try again.", tier, validTiersCommaList)
		}
		isProTier = IsProTier(tier)
	}

	if version == "" {
		version = buildinfo.Current().Version()
	}

	baseUrl := viper.GetString("base_url")
	if len(baseUrl) == 0 {
		baseUrl = AspectBaseUrl(isProTier)
	}

	return &VersionConfig{
		ProTier: isProTier,
		Version: version,
		BaseUrl: baseUrl,
	}, nil
}

func ParseConfigVersion(version string) (string, string, error) {
	if len(version) == 0 {
		return "", "", nil
	}

	tier := ""
	if version[0] < '0' || version[0] > '9' {
		// There is a tier component to the version: <tier> or <tier>/1.2.3
		splits := strings.Split(version, "/")
		if len(splits) == 1 {
			tier = splits[0]
			// Only tier is specified;  version is derived above from running version
		} else if len(splits) == 2 {
			// Both tier and version are specified in the Aspect CLI config
			tier = splits[0]
			version = splits[1]
		} else {
			return "", "", fmt.Errorf("Invalid Aspect CLI version in configuration: '%s'. Version should be [<tier>/]<version> with an optional tier set to one of: %s. Please fix your Aspect CLI configuration and try again.", version, validTiersCommaList)
		}
	}

	return tier, version, nil
}
