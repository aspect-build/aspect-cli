/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package clean

import (
	"bufio"
	"errors"
	"fmt"
	"io/ioutil"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/manifoldco/promptui"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

const (
	skipPromptKey = "clean.skip_prompt"

	ReclaimOption         = "Reclaim disk space for this workspace (same as bazel clean)"
	ReclaimAllOption      = "Reclaim disk space for all Bazel workspaces"
	NonIncrementalOption  = "Prepare to perform a non-incremental build"
	InvalidateReposOption = "Invalidate all repository rules, causing them to recreate external repos"
	WorkaroundOption      = "Workaround inconsistent state in the output tree"

	outputBaseHint = `It's faster to perform a non-incremental build by choosing a different output base.
Instead of running 'clean' you should use the --output_base flag.
Run 'aspect help clean' for more info.
`
	syncHint = `It's faster to invalidate repository rules by using the sync command.
Instead of running 'clean' you should run 'aspect sync --configure'
Run 'aspect help clean' for more info.
`
	fileIssueHint = `Bazel is a correct build tool, and it should not be possible to get inconstent state.
We highly recommend you file a bug reporting this problem so that the offending rule
implementation can be fixed.
`

	rememberLine1 = "You can skip this prompt to make 'aspect clean' behave the same as 'bazel clean'\n"
	rememberLine2 = "Remember this choice and skip the prompt in the future"

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
	bzl               bazel.Bazel
	isInteractiveMode bool

	Behavior   SelectRunner
	Workaround PromptRunner
	Remember   PromptRunner
	Prefs      viper.Viper

	Expunge      bool
	ExpungeAsync bool
}

// New creates a Clean command.
func New(
	streams ioutils.Streams,
	bzl bazel.Bazel,
	isInteractiveMode bool) *Clean {
	return &Clean{
		Streams:           streams,
		isInteractiveMode: isInteractiveMode,
		bzl:               bzl,
	}
}

func NewDefault(streams ioutils.Streams, bzl bazel.Bazel, isInteractive bool) *Clean {
	c := New(
		streams,
		bzl,
		isInteractive)
	c.Behavior = &promptui.Select{
		Label: "Clean can have a few behaviors. Which do you want?",
		Items: []string{
			ReclaimOption,
			ReclaimAllOption,
			NonIncrementalOption,
			InvalidateReposOption,
			WorkaroundOption,
		},
	}
	c.Workaround = &promptui.Prompt{
		Label:     "Temporarily workaround the bug by deleting the output directory",
		IsConfirm: true,
	}
	c.Remember = &promptui.Prompt{
		Label:     rememberLine2,
		IsConfirm: true,
	}
	c.Prefs = *viper.GetViper()
	return c
}

// Run runs the aspect build command.
func (c *Clean) Run(_ *cobra.Command, _ []string) error {
	skip := c.Prefs.GetBool(skipPromptKey)
	if c.isInteractiveMode && !skip {

		_, chosen, err := c.Behavior.Run()

		if err != nil {
			return fmt.Errorf("prompt failed: %w", err)
		}

		switch chosen {

		case ReclaimOption:
			// Allow user to opt-out of our fancy "clean" command and just behave like bazel
			fmt.Fprint(c.Streams.Stdout, rememberLine1)
			if _, err := c.Remember.Run(); err == nil {
				c.Prefs.Set(skipPromptKey, "true")
				if err := c.Prefs.WriteConfig(); err != nil {
					return fmt.Errorf("failed to update config file: %w", err)
				}
			}
		case ReclaimAllOption:
			return c.reclaimAll()
		case NonIncrementalOption:
			fmt.Fprint(c.Streams.Stdout, outputBaseHint)
			return nil
		case InvalidateReposOption:
			fmt.Fprint(c.Streams.Stdout, syncHint)
			return nil
		case WorkaroundOption:
			fmt.Fprint(c.Streams.Stdout, fileIssueHint)
			_, err := c.Workaround.Run()
			if err != nil {
				return fmt.Errorf("prompt failed: %w", err)
			}
		}
	}

	cmd := []string{"clean"}
	if c.Expunge {
		cmd = append(cmd, "--expunge")
	}
	if c.ExpungeAsync {
		cmd = append(cmd, "--expunge_async")
	}
	if exitCode, err := c.bzl.Spawn(cmd); exitCode != 0 {
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
	tempDir, err := ioutil.TempDir("", "tmp_bazel_output")
	if err != nil {
		errors <- fmt.Errorf("failed to create tmp dir: %w", err)
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
	c.bzl.RunCommand([]string{
		"query",
		"//",
		"--build_event_json_file=" + bepLocation,

		// We dont want bazel to print anything to the command line.
		// We are only interested in the BEP output
		"--ui_event_filters=-fatal,-error,-warning,-info,-progress,-debug,-start,-finish,-subcommand,-stdout,-stderr,-pass,-fail,-timeout,-cancelled,-depchecker",
		"--noshow_progress",
	}, streams)

	file, err := os.Open(bepLocation)
	if err != nil {
		errors <- fmt.Errorf("failed to read file: %w", err)
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
					errors <- fmt.Errorf("failed to stat cache directory: %w", err)
					return
				}

				cacheInfo.accessTime = c.GetAccessTime(fileStat)
				cacheInfo.workspaceName = cachePath

				sizeCalcQueue <- cacheInfo
			}
		}
	}

	if err := scanner.Err(); err != nil {
		errors <- fmt.Errorf("failed to read file: %w", err)
		return
	}
}

func (c *Clean) findBazelWorkspaces(
	sizeCalcQueue chan<- bazelDirInfo,
	errors chan<- error,
) {
	bazelBaseDir, currentWorkingBase, err := c.findBazelBaseDir()
	if err != nil {
		errors <- fmt.Errorf("failed to find bazel working dir: %w", err)
		return
	}

	bazelWorkspaces, err := ioutil.ReadDir(bazelBaseDir)
	if err != nil {
		errors <- fmt.Errorf("failed to find bazel workspaces: %w", err)
		return
	}

	// Find bazel workspaces and start processing.
	for _, workspace := range bazelWorkspaces {
		workspaceInfo := bazelDirInfo{
			path:               filepath.Join(bazelBaseDir, workspace.Name()),
			isCurrentWorkspace: workspace.Name() == currentWorkingBase,
			isCache:            false,
		}

		workspaceInfo.accessTime = c.GetAccessTime(workspace)

		execrootFiles, readDirErr := ioutil.ReadDir(filepath.Join(bazelBaseDir, workspace.Name(), "execroot"))
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
		externalDirectories, _ := ioutil.ReadDir(filepath.Join(bazelDir.path, "external"))
		for _, directory := range externalDirectories {
			newPath := c.MoveDirectoryToTmp(bazelDir.path, directory.Name())

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
		if _, err := c.ChangeDirectoryPermissions(bazelDir.path); err != nil {
			waitGroup.Done()
			errors <- fmt.Errorf("failed to delete %q: %w", bazelDir.path, err)
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
		return "", "", fmt.Errorf("failed to find Bazel base dir: %w", err)
	}

	files, err := ioutil.ReadDir(cwd)
	if err != nil {
		return "", "", fmt.Errorf("failed to find Bazel base dir: %w", err)
	}

	for _, file := range files {

		// bazel-bin, bazel-out, etc... will be symlinks, so we can eliminate non-symlinks immediately.
		if file.Mode()&os.ModeSymlink != 0 {
			actualPath, err := os.Readlink(filepath.Join(cwd, file.Name()))
			if err != nil {
				return "", "", fmt.Errorf("failed to find Bazel base dir: %w", err)
			}

			if strings.Contains(actualPath, "bazel") && strings.Contains(actualPath, "/execroot/") {
				execrootBase := strings.Split(actualPath, "/execroot/")[0]
				execrootSplit := strings.Split(execrootBase, "/")
				currentWorkingBase := execrootSplit[len(execrootSplit)-1]
				bazelOutputBase := strings.Join(execrootSplit[:len(execrootSplit)-1], "/")
				return bazelOutputBase, currentWorkingBase, nil
			}
		}
	}

	return "", "", fmt.Errorf("failed to find Bazel base dir: bazel output symlinks not found in directory")
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
