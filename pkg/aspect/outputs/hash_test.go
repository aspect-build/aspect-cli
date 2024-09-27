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

package outputs_test

import (
	"fmt"
	"testing"

	"aspect.build/cli/pkg/aspect/outputs"

	. "github.com/onsi/gomega"
)

func testFixtures(num int) []string {
	r := make([]string, 0, num)
	for i := 0; i < num; i++ {
		r = append(r, fmt.Sprintf("test-fixture-%v", i))
	}
	return r
}

func TestHash(t *testing.T) {
	t.Run("hashMurmur3Sync and hashMurmur3Concurrent return the same hash for the same set of files", func(t *testing.T) {
		g := NewGomegaWithT(t)

		const testTarget1 = "//pkg/aspect/outputs:test_label"
		const testTarget2 = "//pkg/aspect/outputs:test_label"

		hashFiles := make(map[string][]string)
		hashFiles["//:test_label_1"] = testFixtures(1)
		hashFiles["//:test_label_5"] = testFixtures(5)
		hashFiles["//:test_label_9"] = testFixtures(9)

		resultSync, err := outputs.HashLabelFiles(hashFiles, 0)
		g.Expect(err).To(BeNil())
		g.Expect(resultSync["//:test_label_1"]).To(Equal("m3:ZE71kCDqPq6GAqERC0yCeQ=="))
		g.Expect(resultSync["//:test_label_5"]).To(Equal("m3:yBvd/Ck4Gg7BlAQeFGu9iQ=="))
		g.Expect(resultSync["//:test_label_9"]).To(Equal("m3:QAVrLNyY6kHqeiwk0RV8+A=="))

		result1, err := outputs.HashLabelFiles(hashFiles, 1)
		g.Expect(err).To(BeNil())
		g.Expect(resultSync).To(Equal(result1))

		result2, err := outputs.HashLabelFiles(hashFiles, 2)
		g.Expect(err).To(BeNil())
		g.Expect(resultSync).To(Equal(result2))

		result10, err := outputs.HashLabelFiles(hashFiles, 10)
		g.Expect(err).To(BeNil())
		g.Expect(resultSync).To(Equal(result10))

		result100, err := outputs.HashLabelFiles(hashFiles, 100)
		g.Expect(err).To(BeNil())
		g.Expect(resultSync).To(Equal(result100))
	})
}
