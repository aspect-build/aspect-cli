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
	"fmt"
	"io/fs"
	"os"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"

	stdlib_mock "aspect.build/cli/pkg/stdlib/mock"
)

const (
	startDir = "fake_working_directory/foo/bar"
)

func TestWorkspaceFinder(t *testing.T) {
	t.Run("when os.Stat fails, Find fails", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		expectedErr := &NotFoundError{StartDir: startDir}

		finder := &finder{
			osStat: func(s string) (fs.FileInfo, error) {
				return nil, expectedErr
			},
		}
		workspacePath, err := finder.Find(startDir)
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
				osStat: func(s string) (fs.FileInfo, error) {
					return nil, os.ErrNotExist
				},
			}
			workspacePath, err := finder.Find(wd)
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
				osStat: func(s string) (fs.FileInfo, error) {
					return fsFileInfo, nil
				},
			}
			workspacePath, err := finder.Find(startDir)
			g.Expect(workspacePath).To(Equal(startDir))
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
				osStat: func(s string) (fs.FileInfo, error) {
					return fsFileInfo, nil
				},
			}
			workspacePath, err := finder.Find(startDir)
			g.Expect(workspacePath).To(Equal(startDir))
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
				osStat: func(s string) (fs.FileInfo, error) {
					return fsFileInfo, nil
				},
			}
			workspacePath, err := finder.Find(startDir)
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
				osStat: func(s string) (fs.FileInfo, error) {
					return fsFileInfo, nil
				},
			}
			workspacePath, err := finder.Find(startDir)
			g.Expect(workspacePath).To(Equal("fake_working_directory/foo"))
			g.Expect(err).To(BeNil())
		})
	})
}
