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
	"encoding/base64"
	"fmt"
	"io"
	"io/ioutil"
	"os"
	"path/filepath"
	"regexp"
	"sort"
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
)

var (
	diskCacheRegex = regexp.MustCompile(`--disk_cache.+?(\/.+?)"`)
)

type SelectRunner interface {
	Run() (int, string, error)
}

type PromptRunner interface {
	Run() (string, error)
}

type bazelDirInfo struct {
	path               string
	size               float64
	hRSize             float64 // HumanReadableSize
	unit               string
	workspaceName      string
	comparator         float64
	isCurrentWorkspace bool
	accessTime         time.Duration
	processed          bool
	userAsked          bool
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

	// reclaimAll
	bazelDirs       map[string]bazelDirInfo
	bazelDirsMutex  sync.Mutex
	deleteMutex     sync.Mutex
	deleteCounter   int
	chanErrors      *errorSet
	errorChannel    chan error
	deleteChannel   chan string
	sizeCalcChannel chan bazelDirInfo
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
		deleteCounter:     0,
		chanErrors:        &errorSet{nodes: make(map[errorNode]struct{})},
	}
}

func NewDefault(bzl bazel.Bazel, isInteractive bool) *Clean {
	c := New(
		ioutils.DefaultStreams,
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
		Label:     "Temporarily workaround the bug by deleting the output folder",
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
	c.bazelDirs = make(map[string]bazelDirInfo)

	c.sizeCalcChannel = make(chan bazelDirInfo, 64)
	c.deleteChannel = make(chan string, 128)
	c.errorChannel = make(chan error, 64)

	// threads for calculating sizes of directories
	for i := 0; i < 5; i++ {
		go c.directoryProcessor()
	}

	// threads for deleting directories
	for i := 0; i < 5; i++ {
		go c.deleteProcessor()
	}

	// thread for processing errors from the other threads
	go c.errorProcessor()

	// will find disk caches and add them to the processing queue
	c.findDiskCaches()

	// will find bazel workspaces and add them to the processing queue
	c.findBazelWorkspaces()

	var spaceReclaimed float64
	for i := 0; i < len(c.bazelDirs); i++ {
		processedBazelWorkspaceDirectories := c.getProcessedBazelWorkspaceDirectories()

		// getProcessedBazelWorkspaceDirectories will only return if there is at least one directory to remove
		dir := processedBazelWorkspaceDirectories[0]

		label := fmt.Sprintf("Workspace: %s, Age: %s, Size: %.2f %s. Would you like to remove", dir.workspaceName, dir.accessTime, dir.hRSize, dir.unit)
		if dir.isCache {
			label = fmt.Sprintf("Cache: %s, Age: %s, Size: %.2f %s. Would you like to remove", dir.workspaceName, dir.accessTime, dir.hRSize, dir.unit)
		}
		prompt := &promptui.Prompt{
			Label:     label,
			IsConfirm: true,
		}
		_, promptErr := prompt.Run()

		if promptErr == nil {
			fmt.Printf("%s added to the delete queue\n", dir.workspaceName)
			c.manipulateDeleteCounter(1)
			c.deleteChannel <- dir.path
			spaceReclaimed = spaceReclaimed + dir.size
		}

		c.bazelDirsMutex.Lock()
		replacementDir := c.bazelDirs[dir.path]
		replacementDir.userAsked = true
		c.bazelDirs[dir.path] = replacementDir
		c.bazelDirsMutex.Unlock()

		// we dont want to ask the user if they want to continue when there are no more items
		if i < len(c.bazelDirs)-1 {
			_, hRSpaceReclaimed, unit := c.makeBytesHumanReadable(spaceReclaimed)
			prompt2 := &promptui.Prompt{
				Label:     fmt.Sprintf("Space reclaimed: %.2f %s. Would you like to continue", hRSpaceReclaimed, unit),
				IsConfirm: true,
			}
			_, promptErr = prompt2.Run()
			if promptErr != nil {
				break
			}
		}
	}

	// Holding function. Will recursively print and sleep until all deletes have completed
	c.waitForDeletes()

	_, hRSpaceReclaimed, unit := c.makeBytesHumanReadable(spaceReclaimed)
	fmt.Printf("Disk Space Gained: %.2f %s. Exiting...\n", hRSpaceReclaimed, unit)

	// close all the channels
	close(c.sizeCalcChannel)
	close(c.deleteChannel)
	close(c.errorChannel)

	if c.chanErrors.size > 0 {
		return c.chanErrors.generateError()
	}

	return nil
}

func (c *Clean) findDiskCaches() {
	tempDir, err := ioutil.TempDir("", "DirName")
	if err != nil {
		c.errorChannel <- fmt.Errorf("failed to create tmp dir: %w", err)
		return
	}
	defer os.RemoveAll(tempDir)

	bepLocation := tempDir + "/bep.json"

	r, w := io.Pipe()
	decoder := base64.NewDecoder(base64.StdEncoding, r)
	go func() {
		defer w.Close()
		c.bzl.RunCommand([]string{
			"query",
			"//",
			"--build_event_json_file=" + bepLocation,
			"--ui_event_filters=-fatal,-error,-warning,-info,-progress,-debug,-start,-finish,-subcommand,-stdout,-stderr,-pass,-fail,-timeout,-cancelled,-depchecker",
			"--noshow_progress",
		}, w)
	}()

	helpProtoBytes, _ := ioutil.ReadAll(decoder)
	_ = helpProtoBytes

	file, err := os.Open(bepLocation)
	if err != nil {
		c.errorChannel <- fmt.Errorf("failed to read file: %w", err)
		return
	}
	defer file.Close()

	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		text := scanner.Text()
		if strings.Contains(text, "unstructuredCommandLine") {
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
					c.errorChannel <- fmt.Errorf("failed to stat cache folder: %w", err)
					return
				}

				cacheInfo.accessTime = c.GetAccessTime(fileStat)
				cacheInfo.workspaceName = cachePath

				c.bazelDirsMutex.Lock()
				c.bazelDirs[cachePath] = cacheInfo
				c.bazelDirsMutex.Unlock()
				c.sizeCalcChannel <- cacheInfo
			}
		}
	}

	if err := scanner.Err(); err != nil {
		c.errorChannel <- fmt.Errorf("failed to read file: %w", err)
		return
	}
}

func (c *Clean) findBazelWorkspaces() {
	bazelBaseDir, currentWorkingBase, err := c.findBazelBaseDir()
	if err != nil {
		c.errorChannel <- fmt.Errorf("failed to find bazel working dir: %w", err)
		return
	}

	bazelWorkspaces, err := ioutil.ReadDir(bazelBaseDir)
	if err != nil {
		c.errorChannel <- fmt.Errorf("failed to find bazel worksapces: %w", err)
		return
	}

	// find bazel workspaces and start processing
	for _, workspace := range bazelWorkspaces {
		workspaceInfo := bazelDirInfo{
			path:               bazelBaseDir + "/" + workspace.Name(),
			isCurrentWorkspace: workspace.Name() == currentWorkingBase,
			isCache:            false,
		}

		workspaceInfo.accessTime = c.GetAccessTime(workspace)

		execrootFiles, readDirErr := ioutil.ReadDir(bazelBaseDir + "/" + workspace.Name() + "/execroot")
		if readDirErr != nil {
			// install and cache folders will end up here. Currently we dont want to remove these
			continue
		}

		// We expect these folders to have up to 2 files / folders
		//   - Firstly, a file named "DO_NOT_BUILD_HERE"
		//   - Secondly, a folder named after the given workspace
		// We can use the given second folder to determine the name of the workspace. We want this
		// so that we can ask the user if they want to remove a given workspace
		if (len(execrootFiles) == 1 && execrootFiles[0].Name() == "DO_NOT_BUILD_HERE") || len(execrootFiles) > 2 {
			// TODO: Do we want to have an "other" or an "unknown" and remove any that fall here
			continue
		}

		for _, execrootFile := range execrootFiles {
			if execrootFile.Name() != "DO_NOT_BUILD_HERE" {
				workspaceInfo.workspaceName = execrootFile.Name()
			}
		}

		c.bazelDirsMutex.Lock()
		c.bazelDirs[workspaceInfo.path] = workspaceInfo
		c.bazelDirsMutex.Unlock()

		c.sizeCalcChannel <- workspaceInfo
	}
}

func (c *Clean) manipulateDeleteCounter(increment int) {
	c.deleteMutex.Lock()
	c.deleteCounter = c.deleteCounter + increment
	c.deleteMutex.Unlock()
}

func (c *Clean) waitForDeletes() {
	c.waitForDeletesInternal(true)
}

func (c *Clean) waitForDeletesInternal(firstCall bool) {
	c.deleteMutex.Lock()
	counterSize := c.deleteCounter
	c.deleteMutex.Unlock()

	if counterSize > 0 {
		// we need to wait
		if firstCall {
			fmt.Print("Waiting for delete's to complete")
		} else {
			fmt.Print(".")
		}
		time.Sleep(100 * time.Millisecond)
		c.waitForDeletesInternal(false)
		if firstCall {
			fmt.Println("")
		}
	}
}

func (c *Clean) directoryProcessor() {
	for dir := range c.sizeCalcChannel {
		size, hRSize, unit := c.getDirSize(dir.path)

		c.bazelDirsMutex.Lock()

		replacementDir := c.bazelDirs[dir.path]
		replacementDir.size = size
		replacementDir.hRSize = hRSize
		replacementDir.unit = unit
		replacementDir.processed = true
		comparator := replacementDir.accessTime.Hours() * float64(size)

		if replacementDir.isCurrentWorkspace {
			// If we can avoid cleaning the current working directory then maybe we want to do so?
			// If the user has selected this mode then they just want to reclaim resources.
			// Keeping the bazel workspace for the current repo will mean faster build times for that repo.
			// Dividing by 2 so that the current workspace will be listed later to the user.
			comparator = comparator / 2
		}

		replacementDir.comparator = comparator
		c.bazelDirs[dir.path] = replacementDir

		c.bazelDirsMutex.Unlock()
	}
}

func (c *Clean) deleteProcessor() {
	for dir := range c.deleteChannel {

		// We know that there will be an "external" folder that could be deleted in parallel.
		// So we can move those folders to a tmp filepath and add them as seperate deletes that will
		// therefore happen in parallel.
		externalFolders, _ := ioutil.ReadDir(dir + "/external")
		for _, folder := range externalFolders {
			newPath := c.MoveFolderToTmp(dir, folder.Name())

			if newPath != "" {
				c.manipulateDeleteCounter(1)
				c.deleteChannel <- newPath
			}
		}

		// otherwise certain files will throw a permission error when we try to remove them
		_, err := c.ChangeFolderPermissions(dir)
		if err != nil {
			c.errorChannel <- fmt.Errorf("failed to chmod on %s: %w", dir, err)
			c.manipulateDeleteCounter(-1)
			continue
		}

		// remove entire folder
		err = os.RemoveAll(dir)
		if err != nil {
			c.errorChannel <- fmt.Errorf("failed to delete %s: %w", dir, err)
		}
		c.manipulateDeleteCounter(-1)
	}
}

func (c *Clean) errorProcessor() {
	for err := range c.errorChannel {
		c.chanErrors.insert(err)
	}
}

func (c *Clean) findBazelBaseDir() (string, string, error) {
	var bazelOutputBase, currentWorkingBase string

	cwd, err := os.Getwd()

	if err != nil {
		return bazelOutputBase, currentWorkingBase, err
	}

	files, err := ioutil.ReadDir(cwd)
	if err != nil {
		return bazelOutputBase, currentWorkingBase, err
	}

	for _, file := range files {

		// bazel-bin, bazel-out, etc... will be symlinks, so we can eliminate non-symlinks immediately
		if file.Mode()&os.ModeSymlink != 0 {
			actualPath, _ := os.Readlink(cwd + "/" + file.Name())

			if strings.Contains(actualPath, "bazel") && strings.Contains(actualPath, "/execroot/") {
				execrootBase := strings.Split(actualPath, "/execroot/")[0]
				execrootSplit := strings.Split(execrootBase, "/")
				currentWorkingBase = execrootSplit[len(execrootSplit)-1]
				bazelOutputBase = strings.Join(execrootSplit[:len(execrootSplit)-1], "/")
				break
			}
		}
	}

	return bazelOutputBase, currentWorkingBase, err
}

func (c *Clean) getProcessedBazelWorkspaceDirectories() []bazelDirInfo {
	return c.getProcessedBazelWorkspaceDirectoriesInternal(true)
}

func (c *Clean) getProcessedBazelWorkspaceDirectoriesInternal(firstCall bool) []bazelDirInfo {
	c.bazelDirsMutex.Lock()
	bazelWorkspaceDirectories := make([]bazelDirInfo, 0)

	for _, bazelDirectoryInfo := range c.bazelDirs {
		if bazelDirectoryInfo.processed && !bazelDirectoryInfo.userAsked {
			bazelWorkspaceDirectories = append(bazelWorkspaceDirectories, bazelDirectoryInfo)
		}
	}

	c.bazelDirsMutex.Unlock()

	if len(bazelWorkspaceDirectories) == 0 {
		if firstCall {
			fmt.Print("Processing bazel workspaces")
		} else {
			fmt.Print(".")
		}
		time.Sleep(100 * time.Millisecond)
		return c.getProcessedBazelWorkspaceDirectoriesInternal(false)
	}
	if !firstCall {
		fmt.Println("")
	}

	// we want to suggest removing bazel workspaces with the larger comparators first
	sort.Slice(bazelWorkspaceDirectories, func(i, j int) bool {
		return bazelWorkspaceDirectories[j].comparator < bazelWorkspaceDirectories[i].comparator
	})

	return bazelWorkspaceDirectories
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
