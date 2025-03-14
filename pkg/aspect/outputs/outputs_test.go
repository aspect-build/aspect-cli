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

package outputs

import (
	"testing"

	. "github.com/onsi/gomega"
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
)

func TestOutputs(t *testing.T) {
	t.Run("RemoveCobraFlagsFromArgs removes cobra flags", func(t *testing.T) {
		g := NewGomegaWithT(t)

		cmd := &cobra.Command{
			Use: "outputs",
		}

		hash_salt := "some_hash_salt"

		AddFlags(cmd.Flags())

		cmd.Flags().VisitAll(func(f *pflag.Flag) {
			// Need to set this as we arent actually running cobra which would populate this from the actual args.
			if f.Name == "hash_salt" {
				f.Value.Set(hash_salt)
			}
		})

		resultingFlags := RemoveCobraFlagsFromArgs(cmd, []string{"foo", "bar", "--hash_salt", hash_salt, "baz"})

		g.Expect(len(resultingFlags)).To(Equal(3))
		g.Expect(resultingFlags[0]).To(Equal("foo"))
		g.Expect(resultingFlags[1]).To(Equal("bar"))
		g.Expect(resultingFlags[2]).To(Equal("baz"))

		resultingFlags = RemoveCobraFlagsFromArgs(cmd, []string{"foo", "bar", "--hash_salt=" + hash_salt, "baz"})

		g.Expect(len(resultingFlags)).To(Equal(3))
		g.Expect(resultingFlags[0]).To(Equal("foo"))
		g.Expect(resultingFlags[1]).To(Equal("bar"))
		g.Expect(resultingFlags[2]).To(Equal("baz"))

		resultingFlags = RemoveCobraFlagsFromArgs(cmd, []string{"foo", "bar", "baz", "--hash_salt", hash_salt})

		g.Expect(len(resultingFlags)).To(Equal(3))
		g.Expect(resultingFlags[0]).To(Equal("foo"))
		g.Expect(resultingFlags[1]).To(Equal("bar"))
		g.Expect(resultingFlags[2]).To(Equal("baz"))

		resultingFlags = RemoveCobraFlagsFromArgs(cmd, []string{"foo", "bar", "baz", "--hash_salt=" + hash_salt})

		g.Expect(len(resultingFlags)).To(Equal(3))
		g.Expect(resultingFlags[0]).To(Equal("foo"))
		g.Expect(resultingFlags[1]).To(Equal("bar"))
		g.Expect(resultingFlags[2]).To(Equal("baz"))

		resultingFlags = RemoveCobraFlagsFromArgs(cmd, []string{"--hash_salt", hash_salt, "foo", "bar", "baz"})

		g.Expect(len(resultingFlags)).To(Equal(3))
		g.Expect(resultingFlags[0]).To(Equal("foo"))
		g.Expect(resultingFlags[1]).To(Equal("bar"))
		g.Expect(resultingFlags[2]).To(Equal("baz"))

		resultingFlags = RemoveCobraFlagsFromArgs(cmd, []string{"--hash_salt=" + hash_salt, "foo", "bar", "baz"})

		g.Expect(len(resultingFlags)).To(Equal(3))
		g.Expect(resultingFlags[0]).To(Equal("foo"))
		g.Expect(resultingFlags[1]).To(Equal("bar"))
		g.Expect(resultingFlags[2]).To(Equal("baz"))
	})
}
