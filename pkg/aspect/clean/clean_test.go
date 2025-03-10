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

package clean_test

import (
	"context"
	"fmt"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"

	"github.com/aspect-build/aspect-cli/pkg/aspect/clean"
	"github.com/aspect-build/aspect-cli/pkg/bazel/mock"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
)

type confirm struct{}

func (p confirm) Run() (string, error) {
	return "", nil
}

type deny struct{}

func (p deny) Run() (string, error) {
	return "", fmt.Errorf("said no")
}

func TestClean(t *testing.T) {

	t.Run("clean calls bazel clean", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		streams := ioutils.Streams{}
		bzl := mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			RunCommand(streams, nil, "clean").
			Return(nil)

		b := clean.New(streams, bzl)
		g.Expect(b.Run(context.Background(), nil, []string{})).Should(Succeed())
	})
}
