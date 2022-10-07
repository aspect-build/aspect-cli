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
	"strings"

	"github.com/spf13/pflag"
)

func RegisterNoableBool(flags *pflag.FlagSet, name string, value bool, usage string) *bool {
	return RegisterNoableBoolP(flags, name, "", value, usage)
}

// Register a boolean flag that supports the Bazel option parsing.
// https://bazel.build/reference/command-line-reference#option-syntax
// Examples:
//
//	--foo
//	--nofoo
//	--foo=yes
//	--foo=no
//	--foo=1
//	--foo=0
func RegisterNoableBoolP(
	flags *pflag.FlagSet,
	name string,
	shorthand string,
	value bool,
	usage string) *bool {

	result := value
	nb := &noableBool{value: &result}

	flag := &pflag.Flag{
		Name:      name,
		Shorthand: shorthand,
		Usage:     usage,
		Value:     nb,
		DefValue:  nb.String(),
		// The value that will be passed to Set() if no other values are specified.
		NoOptDefVal: "true",
	}
	flags.AddFlag(flag)

	noFlag := &pflag.Flag{
		Name:      "no" + name,
		Shorthand: "",
		Usage:     usage,
		Value:     nb,
		DefValue:  nb.String(),
		// The value that will be passed to Set() if no other values are specified.
		NoOptDefVal: "false",
	}
	flags.AddFlag(noFlag)

	return &result
}

func boolStr(value bool) string {
	return fmt.Sprintf("%t", value)
}

type noableBool struct {
	// The address of the actual value.
	value *bool
}

func (nb *noableBool) Type() string {
	return "bool"
}

func (nb *noableBool) String() string {
	return boolStr(*nb.value)
}

func (nb *noableBool) Set(value string) error {
	switch strings.ToLower(value) {
	case "true", "yes", "1":
		*nb.value = true
	case "false", "no", "0":
		*nb.value = false
	default:
		return fmt.Errorf("invalid bool value %s", value)
	}
	return nil
}
