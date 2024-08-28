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

package workspace

import (
	"errors"
	"testing"

	. "github.com/onsi/gomega"
)

func TestIsNotFoundError(t *testing.T) {
	t.Run("when it is a NotFoundError", func(t *testing.T) {
		g := NewWithT(t)
		err := &NotFoundError{StartDir: "path"}
		g.Expect(IsNotFoundError(err)).To(BeTrue())
	})
	t.Run("when it is not a NotFoundError", func(t *testing.T) {
		g := NewWithT(t)
		err := errors.New("not a NotFoundError")
		g.Expect(IsNotFoundError(err)).To(BeFalse())
	})
}
