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

package run_test

import (
	"context"
	"fmt"
	"strings"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"

	"github.com/aspect-build/aspect-cli/pkg/aspect/run"
	"github.com/aspect-build/aspect-cli/pkg/aspecterrors"
	bazel_mock "github.com/aspect-build/aspect-cli/pkg/bazel/mock"
	"github.com/aspect-build/aspect-cli/pkg/ioutils"
	"github.com/aspect-build/aspect-cli/pkg/plugin/system/bep"
	bep_mock "github.com/aspect-build/aspect-cli/pkg/plugin/system/bep/mock"
)

func TestRun(t *testing.T) {
	t.Run("when the bazel runner fails, the aspect run fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		streams := ioutils.Streams{}
		bzl := bazel_mock.NewMockBazel(ctrl)
		expectErr := &aspecterrors.ExitError{
			Err:      fmt.Errorf("failed to run bazel run"),
			ExitCode: 5,
		}
		bzl.
			EXPECT().
			RunCommand(streams, nil, "run", "//...", "--bes_backend=grpc://127.0.0.1:12345").
			Return(expectErr)
		besBackend := bep_mock.NewMockBESBackend(ctrl)
		besBackend.
			EXPECT().
			Addr().
			Return("grpc://127.0.0.1:12345").
			Times(1)
		besBackend.
			EXPECT().
			Errors().
			Times(1)

		ctx := bep.InjectBESBackend(context.Background(), besBackend)

		b := run.New(streams, bzl)
		err := b.Run(ctx, nil, []string{"//..."})

		g.Expect(err).To(MatchError(expectErr))
	})

	t.Run("when the BES backend contains errors, the aspect run fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		var stderr strings.Builder
		streams := ioutils.Streams{Stderr: &stderr}
		bzl := bazel_mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			RunCommand(streams, nil, "run", "//...", "--bes_backend=grpc://127.0.0.1:12345").
			Return(nil)
		besBackend := bep_mock.NewMockBESBackend(ctrl)
		besBackend.
			EXPECT().
			Addr().
			Return("grpc://127.0.0.1:12345").
			Times(1)
		besBackend.
			EXPECT().
			Errors().
			Return([]error{
				fmt.Errorf("error 1"),
				fmt.Errorf("error 2"),
			}).
			Times(1)

		ctx := bep.InjectBESBackend(context.Background(), besBackend)

		b := run.New(streams, bzl)
		err := b.Run(ctx, nil, []string{"//..."})

		expectedError := fmt.Errorf("2 BES subscriber error(s)")

		g.Expect(err).To(MatchError(expectedError))
		g.Expect(stderr.String()).To(Equal("Error: failed to run 'aspect run' command: error 1\nError: failed to run 'aspect run' command: error 2\n"))
	})

	t.Run("when the bazel runner succeeds, the aspect run succeeds", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		streams := ioutils.Streams{}
		bzl := bazel_mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			RunCommand(streams, nil, "run", "//my/runable:target", "--bes_backend=grpc://127.0.0.1:12345", "--", "myarg").
			Return(nil)
		besBackend := bep_mock.NewMockBESBackend(ctrl)
		besBackend.
			EXPECT().
			Addr().
			Return("grpc://127.0.0.1:12345").
			Times(1)
		besBackend.
			EXPECT().
			Errors().
			Times(1)

		ctx := bep.InjectBESBackend(context.Background(), besBackend)

		b := run.New(streams, bzl)
		err := b.Run(ctx, nil, []string{"//my/runable:target", "--", "myarg"})

		g.Expect(err).To(BeNil())
	})
}
