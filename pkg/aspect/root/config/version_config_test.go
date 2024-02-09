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

package config_test

import (
	"fmt"
	"testing"

	"aspect.build/cli/pkg/aspect/root/config"
	. "github.com/onsi/gomega"
)

func TestParseConfigVersion(t *testing.T) {
	g := NewGomegaWithT(t)
	g.Expect(config.ParseConfigVersion("")).To(Equal(config.VersionTuple{Tier: "", Version: ""}))
	g.Expect(config.ParseConfigVersion("/")).To(Equal(config.VersionTuple{Tier: "", Version: ""}))
	g.Expect(config.ParseConfigVersion("1.2.3")).To(Equal(config.VersionTuple{Tier: "", Version: "1.2.3"}))
	g.Expect(config.ParseConfigVersion("/1.2.3")).To(Equal(config.VersionTuple{Tier: "", Version: "1.2.3"}))
	g.Expect(config.ParseConfigVersion("pro/1.2.3")).To(Equal(config.VersionTuple{Tier: "pro", Version: "1.2.3"}))
	g.Expect(config.ParseConfigVersion("foobar/1.2.3")).To(Equal(config.VersionTuple{Tier: "foobar", Version: "1.2.3"}))
	g.Expect(config.ParseConfigVersion("pro")).To(Equal(config.VersionTuple{Tier: "pro", Version: ""}))
	g.Expect(config.ParseConfigVersion("foobar")).To(Equal(config.VersionTuple{Tier: "foobar", Version: ""}))
	g.Expect(config.ParseConfigVersion("pro/")).To(Equal(config.VersionTuple{Tier: "pro", Version: ""}))
	g.Expect(config.ParseConfigVersion("foobar/")).To(Equal(config.VersionTuple{Tier: "foobar", Version: ""}))

	expectedError := fmt.Errorf("Invalid Aspect CLI version: 'in/valid/1.2.3'. Version should be [<tier>/]<version>.")
	_, err := config.ParseConfigVersion("in/valid/1.2.3")
	g.Expect(err).To(MatchError(expectedError))
}
