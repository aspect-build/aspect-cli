/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package workspace

import (
	"fmt"
	"io/fs"
	"os"
	"testing"

	stdlib_mock "aspect.build/cli/pkg/stdlib/mock"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
)

func TestWorkspaceFinder(t *testing.T) {
	t.Run("when os.Getwd fails, Find fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		expectedErr := fmt.Errorf("os.Getwd failed")

		finder := &finder{
			osGetwd: func() (string, error) {
				return "", expectedErr
			},
		}
		workspacePath, err := finder.Find()
		g.Expect(workspacePath).To(BeEmpty())
		g.Expect(err).To(MatchError(expectedErr))
	})

	t.Run("when os.Stat fails, Find fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		expectedErr := fmt.Errorf("os.Stat failed")

		finder := &finder{
			osGetwd: func() (string, error) {
				return "fake_working_directory/foo/bar", nil
			},
			osStat: func(s string) (fs.FileInfo, error) {
				return nil, expectedErr
			},
		}
		workspacePath, err := finder.Find()
		g.Expect(workspacePath).To(BeEmpty())
		g.Expect(err).To(MatchError(expectedErr))
	})

	t.Run("when a WORKSPACE is not found, Find fails", func(t *testing.T) {
		g := NewGomegaWithT(t)

		// We also make sure that Find doesn't get into an infinite loop.
		wds := []string{
			"/level_1/level_2",
			"/level_1/level_2/level_3",
			"level_1/level_2",
			"level_1",
			"level_1/",
			"/level_1/",
			"/level_1/level_2/",
			".",
			"/",
			"",
		}

		for _, wd := range wds {
			expectedErr := fmt.Errorf("failed to find bazel workspace: the current working directory \"%s\" is not a Bazel workspace", wd)
			finder := &finder{
				osGetwd: func() (string, error) {
					return wd, nil
				},
				osStat: func(s string) (fs.FileInfo, error) {
					return nil, os.ErrNotExist
				},
			}
			workspacePath, err := finder.Find()
			g.Expect(workspacePath).To(BeEmpty())
			g.Expect(err).To(MatchError(expectedErr))
		}
	})

	t.Run("succeeds", func(t *testing.T) {
		t.Run("case 1", func(t *testing.T) {
			g := NewGomegaWithT(t)
			ctrl := gomock.NewController(t)
			defer ctrl.Finish()

			fsFileInfo := stdlib_mock.NewMockFSFileInfo(ctrl)
			fsFileInfo.EXPECT().
				IsDir().
				Return(false).
				Times(1)

			finder := &finder{
				osGetwd: func() (string, error) {
					return "fake_working_directory/foo/bar", nil
				},
				osStat: func(s string) (fs.FileInfo, error) {
					return fsFileInfo, nil
				},
			}
			workspacePath, err := finder.Find()
			g.Expect(workspacePath).To(Equal("fake_working_directory/foo/bar"))
			g.Expect(err).To(BeNil())
		})
		t.Run("case 2", func(t *testing.T) {
			g := NewGomegaWithT(t)
			ctrl := gomock.NewController(t)
			defer ctrl.Finish()

			fsFileInfo := stdlib_mock.NewMockFSFileInfo(ctrl)
			gomock.InOrder(
				fsFileInfo.EXPECT().
					IsDir().
					Return(true).
					Times(1),
				fsFileInfo.EXPECT().
					IsDir().
					Return(false).
					Times(1),
			)

			finder := &finder{
				osGetwd: func() (string, error) {
					return "fake_working_directory/foo/bar", nil
				},
				osStat: func(s string) (fs.FileInfo, error) {
					return fsFileInfo, nil
				},
			}
			workspacePath, err := finder.Find()
			g.Expect(workspacePath).To(Equal("fake_working_directory/foo/bar"))
			g.Expect(err).To(BeNil())
		})
		t.Run("case 3", func(t *testing.T) {
			g := NewGomegaWithT(t)
			ctrl := gomock.NewController(t)
			defer ctrl.Finish()

			fsFileInfo := stdlib_mock.NewMockFSFileInfo(ctrl)
			gomock.InOrder(
				fsFileInfo.EXPECT().
					IsDir().
					Return(true).
					Times(1),
				fsFileInfo.EXPECT().
					IsDir().
					Return(true).
					Times(1),
				fsFileInfo.EXPECT().
					IsDir().
					Return(false).
					Times(1),
			)

			finder := &finder{
				osGetwd: func() (string, error) {
					return "fake_working_directory/foo/bar", nil
				},
				osStat: func(s string) (fs.FileInfo, error) {
					return fsFileInfo, nil
				},
			}
			workspacePath, err := finder.Find()
			g.Expect(workspacePath).To(Equal("fake_working_directory/foo"))
			g.Expect(err).To(BeNil())
		})
		t.Run("case 4", func(t *testing.T) {
			g := NewGomegaWithT(t)
			ctrl := gomock.NewController(t)
			defer ctrl.Finish()

			fsFileInfo := stdlib_mock.NewMockFSFileInfo(ctrl)
			gomock.InOrder(
				fsFileInfo.EXPECT().
					IsDir().
					Return(true).
					Times(1),
				fsFileInfo.EXPECT().
					IsDir().
					Return(true).
					Times(1),
				fsFileInfo.EXPECT().
					IsDir().
					Return(true).
					Times(1),
				fsFileInfo.EXPECT().
					IsDir().
					Return(false).
					Times(1),
			)

			finder := &finder{
				osGetwd: func() (string, error) {
					return "fake_working_directory/foo/bar", nil
				},
				osStat: func(s string) (fs.FileInfo, error) {
					return fsFileInfo, nil
				},
			}
			workspacePath, err := finder.Find()
			g.Expect(workspacePath).To(Equal("fake_working_directory/foo"))
			g.Expect(err).To(BeNil())
		})
	})
}
