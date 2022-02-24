/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package bazel

import (
	"encoding/base64"
	"fmt"
	"io"
	"io/ioutil"
	"strings"
	"time"

	"aspect.build/cli/pkg/logger"
	"github.com/bazelbuild/bazelisk/core"
	"github.com/bazelbuild/bazelisk/repositories"
	"google.golang.org/protobuf/proto"
)

var invocationID string = ""

func generateInvocationID() {
	t := time.Now()

	dateString := stringifyInt(t.Year(), 4) + stringifyInt(int(t.Month()), 2) + stringifyInt(t.Day(), 2)

	nanosecondStr := stringifyInt(t.Nanosecond(), 6)

	timeString := stringifyInt(t.Hour(), 2) +
		stringifyInt(t.Minute(), 2) +
		"-" +
		stringifyInt(t.Second(), 2) +
		nanosecondStr[:2] +
		"-" +
		nanosecondStr[2:6]

	superSpecialHexString := "617370656374" // I wonder what this says when converted back to a string?

	// TODO: Do we want to add the PID to the end here? Can use os.Getpid()
	invocationID = dateString + "-" + timeString + "-" + superSpecialHexString
}

func InvovationID() string {
	if invocationID == "" {
		generateInvocationID()
	}
	return invocationID
}

func SetInvovationID(id string) {
	invocationID = id
}

func stringifyInt(i int, length int) string {
	str := fmt.Sprint(i)
	for len(str) < length {
		str = "0" + str
	}
	return str
}

type Bazel interface {
	SetWorkspaceRoot(workspaceRoot string)
	Spawn(command []string) (int, error)
	RunCommand(command []string, out io.Writer) (int, error)
}

type bazel struct {
	workspaceRoot string
}

func New() Bazel {
	return &bazel{}
}

func (b *bazel) SetWorkspaceRoot(workspaceRoot string) {
	b.workspaceRoot = workspaceRoot
}

func (*bazel) createRepositories() *core.Repositories {
	gcs := &repositories.GCSRepo{}
	gitHub := repositories.CreateGitHubRepo(core.GetEnvOrConfig("BAZELISK_GITHUB_TOKEN"))
	// Fetch LTS releases, release candidates and Bazel-at-commits from GCS, forks and rolling releases from GitHub.
	// TODO(https://github.com/bazelbuild/bazelisk/issues/228): get rolling releases from GCS, too.
	return core.CreateRepositories(gcs, gcs, gitHub, gcs, gitHub, true)
}

// Spawn is similar to the main() function of bazelisk
// see https://github.com/bazelbuild/bazelisk/blob/7c3d9d5/bazelisk.go
func (b *bazel) Spawn(command []string) (int, error) {
	return b.RunCommand(command, nil)
}

func (b *bazel) RunCommand(command []string, out io.Writer) (int, error) {
	repos := b.createRepositories()
	if len(invocationID) < 1 {
		panic("Illegal state: running bazel without invocationID set")
	}
	if len(b.workspaceRoot) < 1 {
		panic("Illegal state: running bazel without the workspaceRoot set")
	}

	if !b.containsFlag(command, "invocation_id") {
		command = append(command, "--invocation_id="+invocationID)
	}

	logger.Command(strings.Join(append([]string{"bazel"}, command...), " "))

	bazelisk := NewBazelisk(b.workspaceRoot)
	exitCode, err := bazelisk.Run(command, repos, out)
	// if at the end of the command then print here
	return exitCode, err
}

func (b *bazel) containsFlag(command []string, flag string) bool {
	for _, cmd := range command {
		if strings.Contains(cmd, fmt.Sprintf("--%s=", flag)) || strings.Contains(cmd, fmt.Sprintf("--%s ", flag)) {
			return true
		}
	}
	return false
}

func (b *bazel) Flags() (map[string]*FlagInfo, error) {
	r, w := io.Pipe()
	decoder := base64.NewDecoder(base64.StdEncoding, r)
	bazelErrs := make(chan error, 1)
	defer close(bazelErrs)
	go func() {
		defer w.Close()
		_, err := b.RunCommand([]string{"help", "flags-as-proto"}, w)
		bazelErrs <- err
	}()

	helpProtoBytes, err := ioutil.ReadAll(decoder)
	if err != nil {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", err)
	}

	if err := <-bazelErrs; err != nil {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", err)
	}

	flagCollection := &FlagCollection{}
	if err := proto.Unmarshal(helpProtoBytes, flagCollection); err != nil {
		return nil, fmt.Errorf("failed to get Bazel flags: %w", err)
	}

	flags := make(map[string]*FlagInfo)
	for i := range flagCollection.FlagInfos {
		flags[*flagCollection.FlagInfos[i].Name] = flagCollection.FlagInfos[i]
	}

	return flags, nil
}
