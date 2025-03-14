/*
 * Copyright 2025 Aspect Build Systems, Inc.
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
)

func TestPaths(t *testing.T) {
	tests := []struct {
		input    string
		expected string
	}{
		{"hello\\sworld", "hello world"},
		{"new\\nline", "new\nline"},
		{"back\\bslash", "back\\slash"},
		{"double\\\\slash", "double\\slash"},
		{"raw\\xsequence", "raw\\xsequence"},
		{"no\\", "no\\"}, // Handles trailing backslash
		{"\\s\\n\\b", " \n\\"},
		{"", ""},
		{"normal text", "normal text"},
		{"\\s\\s\\s", "   "},
	}

	for _, tt := range tests {
		t.Run("unescape handles:"+tt.input, func(t *testing.T) {
			g := NewGomegaWithT(t)
			g.Expect(unescape(tt.input)).To(Equal(tt.expected))
		})
	}
}
