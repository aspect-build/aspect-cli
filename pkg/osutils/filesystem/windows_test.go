//go:build windows

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

package filesystem_test

import (
	"syscall"
	"testing"
	"time"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"

	"github.com/aspect-build/aspect-cli/pkg/osutils/filesystem"
	stdlib_mock "github.com/aspect-build/aspect-cli/pkg/stdlib/mock"
)

func TestWindowsOsUtils(t *testing.T) {
	t.Run("GetAccessTime runs successfully", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		fsFileInfo := stdlib_mock.NewMockFSFileInfo(ctrl)

		timespec := syscall.Timespec{
			Sec:  0,
			Nsec: 0,
		}

		alternateSysInfo := syscall.Stat_t{
			Atimespec: timespec,
			Mtimespec: timespec,
			Ctimespec: timespec,
		}

		gomock.InOrder(
			fsFileInfo.EXPECT().
				Sys().
				Return(&alternateSysInfo).
				Times(3),
		)

		fakeFirstTime := time.Date(2022, time.Month(2), 21, 1, 10, 30, 0, time.UTC)
		fakeSecondTime := time.Date(2022, time.Month(2), 21, 1, 10, 30, 0, time.UTC)

		fakeDuration := fakeSecondTime.Sub(fakeFirstTime)

		o := filesystem.Filesystem{}
		o.TimeSince = func(t time.Time) time.Duration {
			return fakeDuration
		}
		o.TimeUnix = func(sec int64, nsec int64) time.Time {
			return fakeFirstTime
		}
		g.Expect(o.GetAccessTime(fsFileInfo)).To(Equal(fakeDuration))

	})

	t.Run("GetAccessTime always returns the shortest duration", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		fsFileInfo := stdlib_mock.NewMockFSFileInfo(ctrl)

		alternateSysInfo := syscall.Stat_t{
			Atim: syscall.Timespec{
				Sec:  1,
				Nsec: 0,
			},
			Mtim: syscall.Timespec{
				Sec:  2,
				Nsec: 0,
			},
			Ctim: syscall.Timespec{
				Sec:  3,
				Nsec: 0,
			},
		}

		gomock.InOrder(
			fsFileInfo.EXPECT().
				Sys().
				Return(&alternateSysInfo).
				Times(9),
		)

		fakeFirstTime := time.Date(2022, time.Month(2), 21, 1, 10, 30, 0, time.UTC)
		fakeSecondTime := fakeFirstTime.Add(time.Second * 60)
		fakeThirdTime := fakeSecondTime.Add(time.Second * 120)
		fakeFourthTime := fakeThirdTime.Add(time.Second * 180)

		fakeShortDuration := fakeSecondTime.Sub(fakeFirstTime)
		fakeMediumDuration := fakeThirdTime.Sub(fakeFirstTime)
		fakeLongDuration := fakeFourthTime.Sub(fakeFirstTime)

		// Short Duration First
		osutilsShortFirst := filesystem.Filesystem{}
		osutilsShortFirst.TimeSince = func(t time.Time) time.Duration {
			if t == fakeFirstTime {
				return fakeShortDuration
			} else if t == fakeSecondTime {
				return fakeMediumDuration
			} else {
				return fakeLongDuration
			}
		}
		osutilsShortFirst.TimeUnix = func(sec int64, nsec int64) time.Time {
			if sec == 1 {
				return fakeFirstTime
			} else if sec == 2 {
				return fakeSecondTime
			} else {
				return fakeThirdTime
			}
		}
		g.Expect(osutilsShortFirst.GetAccessTime(fsFileInfo)).To(Equal(fakeShortDuration))

		// Short Duration Second
		osutilsShortSecond := filesystem.Filesystem{}
		osutilsShortSecond.TimeSince = func(t time.Time) time.Duration {
			if t == fakeFirstTime {
				return fakeMediumDuration
			} else if t == fakeSecondTime {
				return fakeShortDuration
			} else {
				return fakeLongDuration
			}
		}
		osutilsShortSecond.TimeUnix = func(sec int64, nsec int64) time.Time {
			if sec == 1 {
				return fakeFirstTime
			} else if sec == 2 {
				return fakeSecondTime
			} else {
				return fakeThirdTime
			}
		}
		g.Expect(osutilsShortSecond.GetAccessTime(fsFileInfo)).To(Equal(fakeShortDuration))

		// Short Duration Third
		osutilsShortThird := filesystem.Filesystem{}
		osutilsShortThird.TimeSince = func(t time.Time) time.Duration {
			if t == fakeFirstTime {
				return fakeMediumDuration
			} else if t == fakeSecondTime {
				return fakeLongDuration
			} else {
				return fakeShortDuration
			}
		}
		osutilsShortThird.TimeUnix = func(sec int64, nsec int64) time.Time {
			if sec == 1 {
				return fakeFirstTime
			} else if sec == 2 {
				return fakeSecondTime
			} else {
				return fakeThirdTime
			}
		}
		g.Expect(osutilsShortThird.GetAccessTime(fsFileInfo)).To(Equal(fakeShortDuration))
	})

	t.Run("ChangeDirectoryPermissions runs successfully", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		o := filesystem.Filesystem{}

		_, err := o.ChangeDirectoryPermissions(fakeFileFolder, fakeFilePermissions)
		g.Expect(err).To(BeNil())
	})

	t.Run("MoveDirectoryToTmp runs successfully", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		o := filesystem.Filesystem{}

		tmpDir, err := o.MoveDirectoryToTmp(fakeDirectory, fakeNewTmpDirName)
		g.Expect(err).To(BeNil())
	})

}
