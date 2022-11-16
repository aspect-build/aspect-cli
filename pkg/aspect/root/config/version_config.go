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

type VersionTuple struct {
	Tier    string
	Version string
}

type VersionConfig struct {
	VersionTuple
	Configured bool
	AutoTier   bool
	BaseUrl    string
	IsProTier  bool
}

func GetVersionConfig() (*VersionConfig, error) {
	versionString := viper.GetString("version")
	versionTuple, err := ParseConfigVersion(versionString)
	if err != nil {
		return nil, fmt.Errorf("Unrecognized Aspect CLI tier in version in configuration: '%s'. Version should be [<tier>/]<version> with an optional tier set to one of: %s. Please fix your Aspect CLI configuration and try again.", versionString, validTiersCommaList)
	}

	result := VersionConfig{
		VersionTuple: versionTuple,
		Configured:   versionTuple.Version != "" || versionTuple.Tier != "",
	}

	isProTier := false
	if versionTuple.Tier == "" {
		if buildinfo.Current().IsAspectPro {
			result.Tier = "pro"
			result.IsProTier = true
		}
	} else {
		if !IsValidTier(versionTuple.Tier) {
			return nil, fmt.Errorf("Unrecognized Aspect CLI tier in version in configuration: '%s'. Version should be [<tier>/]<version> with an optional tier set to one of: %s. Please fix your Aspect CLI configuration and try again.", versionTuple.Tier, validTiersCommaList)
		}
		result.IsProTier = IsProTier(versionTuple.Tier)
	}

	if result.Version == "" {
		result.Version = buildinfo.Current().Version()
	}

	result.BaseUrl = viper.GetString("base_url")
	if len(result.BaseUrl) == 0 {
		result.BaseUrl = AspectBaseUrl(isProTier)
	}

	return &result, nil
}

func ParseConfigVersion(version string) (VersionTuple, error) {
	result := VersionTuple{"", ""}
	if len(version) == 0 {
		return result, nil
	}

	if version[0] >= '0' && version[0] <= '9' {
		// Version is numeric with no tier component
		result.Version = version
	} else {
		// There is a tier component to the version: <tier> or <tier>/1.2.3
		splits := strings.Split(version, "/")
		if len(splits) == 1 {
			result.Tier = splits[0]
			// Only tier is specified;  version is derived above from running version
		} else if len(splits) == 2 {
			// Both tier and version are specified in the Aspect CLI config
			result.Tier = splits[0]
			result.Version = splits[1]
		} else {
			return result, fmt.Errorf("Invalid Aspect CLI version: '%s'. Version should be [<tier>/]<version>.", version)
		}
	}

	return result, nil
}
