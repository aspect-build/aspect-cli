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

package flags_test

import (
	"fmt"
	"testing"

	"github.com/aspect-build/aspect-cli/pkg/aspect/root/flags"
	. "github.com/onsi/gomega"
	"github.com/spf13/pflag"
)

func assertMsg(arg string) string {
	return fmt.Sprintf("parsing '%s'", arg)
}

func doBoolFlagTest(g *WithT, initial, expected bool, arg string) {
	flagSet := pflag.NewFlagSet("test", pflag.ContinueOnError)
	boolValuePtr := flags.RegisterNoableBoolP(flagSet, "foo", "f", initial, "this is a boolean flag")
	g.Expect(*boolValuePtr).To(Equal(initial))

	msg := assertMsg(arg)
	err := flagSet.Parse([]string{arg})
	g.Expect(err).ToNot(HaveOccurred(), msg)
	g.Expect(*boolValuePtr).To(Equal(expected), msg)
}

func doInvalidBoolFlagTest(g *WithT, arg string, expectedErrMsgSubstring string) {
	flagSet := pflag.NewFlagSet("test", pflag.ContinueOnError)
	flags.RegisterNoableBoolP(flagSet, "foo", "f", false, "this is a boolean flag")

	msg := assertMsg(arg)
	err := flagSet.Parse([]string{arg})
	g.Expect(err).To(HaveOccurred(), msg)
	if len(expectedErrMsgSubstring) > 0 {
		g.Expect(err.Error()).To(ContainSubstring(expectedErrMsgSubstring), msg)
	}
}

func TestNoableBool(t *testing.T) {
	g := NewWithT(t)
	// From Bazel doc
	// https://bazel.build/reference/command-line-reference#option-syntax
	doBoolFlagTest(g, false, true, "--foo")
	doBoolFlagTest(g, true, false, "--nofoo")
	doBoolFlagTest(g, false, true, "--foo=true")
	doBoolFlagTest(g, true, false, "--foo=false")
	doBoolFlagTest(g, false, true, "--foo=yes")
	doBoolFlagTest(g, true, false, "--foo=no")
	doBoolFlagTest(g, false, true, "--foo=1")
	doBoolFlagTest(g, true, false, "--foo=0")
	doBoolFlagTest(g, false, true, "-f")
	doBoolFlagTest(g, false, true, "--foo=True")
	doBoolFlagTest(g, false, true, "--foo=YES")
	doBoolFlagTest(g, true, false, "--foo=nO")

	doInvalidBoolFlagTest(g, "--foo=hello", "invalid bool value 'hello'")
	doInvalidBoolFlagTest(g, "--nofoo=yes", "invalid no flag value 'yes'")
	doInvalidBoolFlagTest(g, "--nonofoo", "")
}
