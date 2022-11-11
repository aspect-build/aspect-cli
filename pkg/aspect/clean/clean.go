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

package clean

import (
	"bufio"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/manifoldco/promptui"
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/osutils/filesystem"
)

const (
	unstructuredArgsBEPKey = "unstructuredCommandLine"
)

var diskCacheRegex = regexp.MustCompile(`--disk_cache.+?(\/.+?)"`)

type SelectRunner interface {
	Run() (int, string, error)
}

type PromptRunner interface {
	Run() (string, error)
}

type bazelDirInfo struct {
	path               string
	size               float64
	humanReadableSize  float64
	unit               string
	workspaceName      string
	comparator         float64
	isCurrentWorkspace bool
	accessTime         time.Duration
	processed          bool
	isCache            bool
}

// Clean represents the aspect clean command.
type Clean struct {
	ioutils.Streams
	bzl bazel.Bazel

	Filesystem filesystem.Filesystem
}

// New creates a Clean command.
func New(
	streams ioutils.Streams,
	bzl bazel.Bazel) *Clean {
	return &Clean{
		Streams: streams,
		bzl:     bzl,
	}
}

func NewDefault(streams ioutils.Streams, bzl bazel.Bazel) *Clean {
	c := New(streams, bzl)
	c.Filesystem = filesystem.NewDefault()
	return c
}

// Run runs the aspect build command.
func (c *Clean) Run(cmd *cobra.Command, args []string) error {
	cleanAll := false

	// TODO: move separation of flags and arguments to a high level of abstraction
	flags := make([]string, 0)
	for i := 0; i < len(args); i++ {
		if args[i] == "all" {
			cleanAll = true
			continue
		}
		flags = append(flags, args[i])
	}

	if cleanAll {
		return c.reclaimAll()
	}

	bazelCmd := []string{"clean"}
	bazelCmd = append(bazelCmd, flags...)
	if exitCode, err := c.bzl.RunCommand(c.Streams, bazelCmd...); exitCode != 0 {
		err = &aspecterrors.ExitError{
			Err:      err,
			ExitCode: exitCode,
		}
		return err
	}

	return nil
}

func (c *Clean) reclaimAll() error {
	sizeCalcQueue := make(chan bazelDirInfo, 64)
	confirmationQueue := make(chan bazelDirInfo, 64)
	deleteQueue := make(chan bazelDirInfo, 128)
	sizeQueue := make(chan float64, 64)
	errorQueue := make(chan error, 64)

	var sizeCalcWaitGroup sync.WaitGroup
	var confirmationWaitGroup sync.WaitGroup
	var deleteWaitGroup sync.WaitGroup
	var sizeWaitGroup sync.WaitGroup
	var errorWaitGroup sync.WaitGroup

	errors := errorSet{nodes: make(map[errorNode]struct{})}

	// Goroutine for processing errors from the other threads.
	go c.errorProcessor(errorQueue, &errorWaitGroup, &errors)

	// Goroutines for calculating sizes of directories.
	for i := 0; i < 5; i++ {
		go c.sizeCalculator(sizeCalcQueue, confirmationQueue, &sizeCalcWaitGroup)
	}

	// Goroutines for deleting directories.
	// We dont add to the wait group here as deleteProcessor will add to the deleteWaitGroup itself.
	// This is due to deleteProcessor breaking up its deletes and adding subdirectories to the
	// deleteQueue as it does the deleting. Where the other wait groups ensure all their goroutines
	// have finished processing, deleteWaitGroup ensures deleteQueue is empty before the program exits.
	for i := 0; i < 8; i++ {
		go c.deleteProcessor(deleteQueue, &deleteWaitGroup, sizeQueue, errorQueue)
	}

	go c.sizePrinter(sizeQueue, &sizeWaitGroup)

	// Goroutine for prompting the user to confirm deletion.
	go c.confirmationActor(confirmationQueue, deleteQueue, &confirmationWaitGroup, &deleteWaitGroup)

	// Find disk caches and add them to the sizeCalculator queue.
	c.findDiskCaches(sizeCalcQueue, errorQueue)

	// Find bazel workspaces and add them to the sizeCalculator queue.
	c.findBazelWorkspaces(sizeCalcQueue, errorQueue)

	// Since the directories are added to sizeCalcQueue synchronously, so we can
	// close the channel before waiting for the calculations to complete.
	close(sizeCalcQueue)
	sizeCalcWaitGroup.Wait()

	// Since the confirmationQueue will be filled before sizeCalcWaitGroup completes,
	// we can close the channel before waiting.
	close(confirmationQueue)
	confirmationWaitGroup.Wait()

	// Since the deleteQueue can add to itself to improve speeds. We need to wait for
	// all deletes to complete before we can close the channel.
	deleteWaitGroup.Wait()
	close(deleteQueue)

	// Since the sizeQueue will contain all the sizes once the deletes are completed,
	// we can close the chan before we wait.
	close(sizeQueue)
	sizeWaitGroup.Wait()

	// All the errors we will receive will be in the errorQueue at this point,
	// so we can close the queue before waiting.
	close(errorQueue)
	errorWaitGroup.Wait()

	if errors.size > 0 {
		return errors.generateError()
	}

	return nil
}

func (c *Clean) confirmationActor(
	directories <-chan bazelDirInfo,
	deleteQueue chan<- bazelDirInfo,
	confirmationWaitGroup *sync.WaitGroup,
	deleteWaitGroup *sync.WaitGroup,
) {
	confirmationWaitGroup.Add(1)
	for bazelDir := range directories {
		var label string
		if bazelDir.isCache {
			label = fmt.Sprintf("Cache: %s, Age: %s, Size: %.2f %s. Would you like to remove?", bazelDir.workspaceName, bazelDir.accessTime, bazelDir.humanReadableSize, bazelDir.unit)
		} else {
			label = fmt.Sprintf("Workspace: %s, Age: %s, Size: %.2f %s. Would you like to remove?", bazelDir.workspaceName, bazelDir.accessTime, bazelDir.humanReadableSize, bazelDir.unit)
		}

		promptRemove := &promptui.Prompt{
			Label:     label,
			IsConfirm: true,
		}

		if _, err := promptRemove.Run(); err == nil {
			fmt.Fprintf(c.Streams.Stdout, "%s added to the delete queue\n", bazelDir.workspaceName)
			deleteWaitGroup.Add(1)
			deleteQueue <- bazelDir
		} else {
			// promptui.ErrInterrupt is the error returned when SIGINT is received.
			// This is most likely due to the user hitting ctrl+c in the terminal.
			if errors.Is(err, promptui.ErrInterrupt) {
				// We allow the program to gracefully exit in such case.
				break
			}
		}

		promptContinue := &promptui.Prompt{
			Label:     "Would you like to continue?",
			IsConfirm: true,
		}
		if _, err := promptContinue.Run(); err != nil || errors.Is(err, promptui.ErrInterrupt) {
			break
		}
	}
	confirmationWaitGroup.Done()
}

func (c *Clean) findDiskCaches(
	sizeCalcQueue chan<- bazelDirInfo,
	errors chan<- error,
) {
	tempDir, err := os.MkdirTemp("", "tmp_bazel_output")
	if err != nil {
		errors <- fmt.Errorf("failed to find disk caches: failed to create tmp dir: %w", err)
		return
	}
	defer os.RemoveAll(tempDir)

	bepLocation := filepath.Join(tempDir, "bep.json")

	streams := ioutils.Streams{
		Stdin:  nil,
		Stdout: nil,
		Stderr: nil,
	}

	// Running an invalid query should ensure that repository rules are not executed.
	// However, bazel will still emit its BEP containing the flag that we are interested in.
	// This will ensure it returns quickly and allows us to easily access said flag.
	c.bzl.RunCommand(
		streams,
		"query",
		"//",
		"--build_event_json_file="+bepLocation,

		// We dont want bazel to print anything to the command line.
		// We are only interested in the BEP output
		"--ui_event_filters=-fatal,-error,-warning,-info,-progress,-debug,-start,-finish,-subcommand,-stdout,-stderr,-pass,-fail,-timeout,-cancelled,-depchecker",
		"--noshow_progress",
	)

	file, err := os.Open(bepLocation)
	if err != nil {
		errors <- fmt.Errorf("failed to find disk caches: failed to open BEP file: %w", err)
		return
	}
	defer file.Close()

	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		text := scanner.Text()
		if strings.Contains(text, unstructuredArgsBEPKey) {
			result := diskCacheRegex.FindAllStringSubmatch(text, -1)
			for i := range result {
				cachePath := result[i][1]

				cacheInfo := bazelDirInfo{
					path:               cachePath,
					isCurrentWorkspace: false,
					isCache:            true,
				}
				fileStat, err := os.Stat(cachePath)

				if err != nil {
					errors <- fmt.Errorf("failed to find disk caches: failed to stat potential cache: %w", err)
					return
				}

				cacheInfo.accessTime = c.Filesystem.GetAccessTime(fileStat)
				cacheInfo.workspaceName = cachePath

				sizeCalcQueue <- cacheInfo
			}
		}
	}

	if err := scanner.Err(); err != nil {
		errors <- fmt.Errorf("failed to find disk caches: failed to read BEP file: %w", err)
		return
	}
}

func (c *Clean) findBazelWorkspaces(
	sizeCalcQueue chan<- bazelDirInfo,
	errors chan<- error,
) {
	bazelBaseDir, currentWorkingBase, err := c.findBazelBaseDir()
	if err != nil {
		errors <- fmt.Errorf("failed to find bazel workspaces: failed to find bazel base directory: %w", err)
		return
	}

	bazelWorkspaces, err := os.ReadDir(bazelBaseDir)
	if err != nil {
		errors <- fmt.Errorf("failed to find bazel workspaces: failed to read bazel base directory: %w", err)
		return
	}

	// Find bazel workspaces and start processing.
	for _, workspace := range bazelWorkspaces {
		workspaceInfo := bazelDirInfo{
			path:               filepath.Join(bazelBaseDir, workspace.Name()),
			isCurrentWorkspace: workspace.Name() == currentWorkingBase,
			isCache:            false,
		}

		wkFileInfo, err := workspace.Info()
		if err != nil {
			errors <- fmt.Errorf("failed to find bazel workspaces: failed to retrieve file info: %w", err)
			return
		}
		workspaceInfo.accessTime = c.Filesystem.GetAccessTime(wkFileInfo)

		execrootFiles, readDirErr := os.ReadDir(filepath.Join(bazelBaseDir, workspace.Name(), "execroot"))
		if readDirErr != nil {
			// The install and cache directories will end up here.We must not remove these
			continue
		}

		// We expect these directories to have up to 2 files / directories:
		//   - Firstly, a file named "DO_NOT_BUILD_HERE".
		//   - Secondly, a directory named after the given workspace.
		// We can use the given second directory to determine the name of the workspace. We want this
		// so that we can ask the user if they want to remove a given workspace.
		if (len(execrootFiles) == 1 && execrootFiles[0].Name() == "DO_NOT_BUILD_HERE") || len(execrootFiles) > 2 {
			// TODO: Only ask the user if they want to remove unknown workspace once.
			// https://github.com/aspect-build/aspect-cli/issues/208
			workspaceInfo.workspaceName = "Unknown Workspace"
		} else {
			for _, execrootFile := range execrootFiles {
				if execrootFile.Name() != "DO_NOT_BUILD_HERE" {
					workspaceInfo.workspaceName = execrootFile.Name()
				}
			}
		}

		sizeCalcQueue <- workspaceInfo
	}
}

func (c *Clean) sizeCalculator(
	in <-chan bazelDirInfo,
	out chan<- bazelDirInfo,
	waitGroup *sync.WaitGroup,
) {
	waitGroup.Add(1)

	for bazelDir := range in {
		size, humanReadableSize, unit := c.getDirSize(bazelDir.path)

		bazelDir.size = size
		bazelDir.humanReadableSize = humanReadableSize
		bazelDir.unit = unit
		bazelDir.processed = true
		comparator := bazelDir.accessTime.Hours() * float64(size)

		if bazelDir.isCurrentWorkspace {
			// If we can avoid cleaning the current working directory then maybe we want to do so?
			// If the user has selected this mode then they just want to reclaim resources.
			// Keeping the bazel workspace for the current repo will mean faster build times for that repo.
			// Dividing by 2 so that the current workspace will be listed later to the user.
			comparator = comparator / 2
		}

		bazelDir.comparator = comparator

		out <- bazelDir
	}

	waitGroup.Done()
}

func (c *Clean) deleteProcessor(
	deleteQueue chan bazelDirInfo,
	waitGroup *sync.WaitGroup,
	sizeQueue chan<- float64,
	errors chan<- error,
) {
	for bazelDir := range deleteQueue {

		// We know that there will be an "external" directory that could be deleted in parallel.
		// So we can move those directories to a tmp filepath and add them as seperate deletes that will
		// therefore happen in parallel.
		externalDirectories, _ := os.ReadDir(filepath.Join(bazelDir.path, "external"))
		for _, directory := range externalDirectories {
			newPath, err := c.Filesystem.MoveDirectoryToTmp(bazelDir.path, directory.Name())

			if err != nil {
				errors <- fmt.Errorf("failed to delete %q: failed to move directory to tmp: %w", bazelDir.path, err)
				continue
			}

			if newPath != "" {
				waitGroup.Add(1)
				deleteQueue <- bazelDirInfo{
					path: newPath,
					// Size is used to calculate how much space has been reclaimed.
					// This bazelDirInfo is for a subdirectory we want to delete in parallel.
					// Rather than calculate the size of each subdirectory we can just use the
					// already calculate size of the parent.
					size: 0,
				}
			}
		}

		// The permissions set in the directories being removed don't allow write access,
		// so we change the permissions before removing those directories.
		if _, err := c.Filesystem.ChangeDirectoryPermissions(bazelDir.path, "0777"); err != nil {
			waitGroup.Done()
			errors <- fmt.Errorf("failed to delete %q: failed to change permissions: %w", bazelDir.path, err)
			continue
		}

		// Remove the entire directory tree.
		if err := os.RemoveAll(bazelDir.path); err != nil {
			waitGroup.Done()
			errors <- fmt.Errorf("failed to delete %q: %w", bazelDir.path, err)
			continue
		}

		sizeQueue <- bazelDir.size

		waitGroup.Done()
	}
}

func (c *Clean) errorProcessor(errorQueue <-chan error, waitGroup *sync.WaitGroup, errors *errorSet) {
	waitGroup.Add(1)
	for err := range errorQueue {
		errors.insert(err)
	}
	waitGroup.Done()
}

func (c *Clean) sizePrinter(sizeQueue <-chan float64, waitGroup *sync.WaitGroup) {
	waitGroup.Add(1)

	var totalSize float64 = 0

	for size := range sizeQueue {
		totalSize = totalSize + size
	}

	_, hRSpaceReclaimed, unit := c.makeBytesHumanReadable(totalSize)
	fmt.Fprintf(c.Streams.Stdout, "Space reclaimed: %.2f%s\n", hRSpaceReclaimed, unit)

	waitGroup.Done()
}

func (c *Clean) findBazelBaseDir() (string, string, error) {
	cwd, err := os.Getwd()
	if err != nil {
		return "", "", fmt.Errorf("failed to find Bazel base directory: failed to get current working directory: %w", err)
	}

	files, err := os.ReadDir(cwd)
	if err != nil {
		return "", "", fmt.Errorf("failed to find Bazel base directory: failed to read current working directory %w", err)
	}

	for _, dirEntry := range files {
		file, err := dirEntry.Info()
		if err != nil {
			return "", "", err
		}
		// bazel-bin, bazel-out, etc... will be symlinks, so we can eliminate non-symlinks immediately.
		if file.Mode()&os.ModeSymlink != 0 {
			actualPath, err := os.Readlink(filepath.Join(cwd, file.Name()))
			if err != nil {
				return "", "", fmt.Errorf("failed to find Bazel base directory: failed to follow symlink: %w", err)
			}

			normalizedPath := filepath.ToSlash(actualPath)
			if strings.Contains(normalizedPath, "bazel") && strings.Contains(normalizedPath, "/execroot/") {
				execrootBase := strings.Split(normalizedPath, "/execroot/")[0]
				execrootSplit := strings.Split(execrootBase, "/")
				currentWorkingBase := execrootSplit[len(execrootSplit)-1]
				bazelOutputBase := strings.Join(execrootSplit[:len(execrootSplit)-1], "/")
				return bazelOutputBase, currentWorkingBase, nil
			}
		}
	}

	return "", "", fmt.Errorf("failed to find Bazel base directory: bazel output symlinks not found in directory")
}

func (c *Clean) getDirSize(path string) (float64, float64, string) {
	var size float64

	filepath.Walk(path, func(path string, file os.FileInfo, err error) error {
		if !file.IsDir() {
			size += float64(file.Size())
		}

		return nil
	})

	return c.makeBytesHumanReadable(size)
}

func (c *Clean) makeBytesHumanReadable(bytes float64) (float64, float64, string) {
	humanReadable, unit := c.makeBytesHumanReadableInternal(bytes, "bytes")
	return bytes, humanReadable, unit
}

func (c *Clean) makeBytesHumanReadableInternal(bytes float64, unit string) (float64, string) {
	if bytes < 1024 {
		return bytes, unit
	}

	bytes = bytes / 1024

	switch unit {
	case "bytes":
		unit = "KB"
	case "KB":
		unit = "MB"
	case "MB":
		unit = "GB"
	case "GB":
		unit = "TB"
	case "TB":
		unit = "PB"
	}

	if bytes >= 1024 {
		return c.makeBytesHumanReadableInternal(bytes, unit)
	}

	return bytes, unit
}

type errorSet struct {
	head  *errorNode
	tail  *errorNode
	nodes map[errorNode]struct{}
	size  int
}

func (s *errorSet) generateError() error {
	var err error
	for node := s.head; node != nil; node = node.next {
		if err == nil {
			err = node.err
		} else {
			err = fmt.Errorf("%s, %w", err, node.err)
		}
	}
	return err
}

func (s *errorSet) insert(err error) {
	node := errorNode{
		err: err,
	}
	if _, exists := s.nodes[node]; !exists {
		s.nodes[node] = struct{}{}
		if s.head == nil {
			s.head = &node
		} else {
			s.tail.next = &node
		}
		s.tail = &node
		s.size++
	}
}

type errorNode struct {
	next *errorNode
	err  error
}
