/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package run_test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"

	"aspect.build/cli/pkg/aspect/run"
	"aspect.build/cli/pkg/aspecterrors"
	bazel_mock "aspect.build/cli/pkg/bazel/mock"
	"aspect.build/cli/pkg/ioutils"
	bep_mock "aspect.build/cli/pkg/plugin/system/bep/mock"
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
			Spawn([]string{"run", "--bes_backend=grpc://127.0.0.1:12345", "//..."}, streams).
			Return(expectErr.ExitCode, expectErr.Err)
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

		b := run.New(streams, bzl)
		err := b.Run([]string{"//..."}, besBackend)

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
			Spawn([]string{"run", "--bes_backend=grpc://127.0.0.1:12345", "//..."}, streams).
			Return(0, nil)
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

		b := run.New(streams, bzl)
		err := b.Run([]string{"//..."}, besBackend)

		g.Expect(err).To(MatchError(&aspecterrors.ExitError{ExitCode: 1}))
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
			Spawn([]string{"run", "--bes_backend=grpc://127.0.0.1:12345", "//..."}, streams).
			Return(0, nil)
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

		b := run.New(streams, bzl)
		err := b.Run([]string{"//..."}, besBackend)

		g.Expect(err).To(BeNil())
	})
}
