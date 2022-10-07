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
	"log"
	"strings"
	"testing"

	"aspect.build/cli/pkg/aspect/root/flags"
	. "github.com/onsi/gomega"
	"github.com/spf13/pflag"
)

func doBoolFlagTest(g *WithT, initial, expected bool, args ...string) {
	flagSet := pflag.NewFlagSet("test", pflag.ContinueOnError)
	boolValuePtr := flags.RegisterNoableBool(flagSet, "foo", false, "this is a boolean flag")
	*boolValuePtr = initial

	// DEBUG BEGIN
	log.Printf("*** CHUCK: =====")
	// DEBUG END

	msg := "parsing '" + strings.Join(args, " ") + "'"
	err := flagSet.Parse(args)
	g.Expect(err).ToNot(HaveOccurred(), msg)
	g.Expect(*boolValuePtr).To(Equal(expected), msg)
}

func TestNoableBool(t *testing.T) {
	g := NewWithT(t)
	// From Bazel doc
	// https://bazel.build/reference/command-line-reference#option-syntax
	doBoolFlagTest(g, false, true, "--foo")
	doBoolFlagTest(g, true, false, "--nofoo")
	doBoolFlagTest(g, false, true, "--foo=yes")
	doBoolFlagTest(g, true, false, "--foo=no")
	doBoolFlagTest(g, false, true, "--foo=1")
	doBoolFlagTest(g, true, false, "--foo=0")
}
