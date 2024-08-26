/*
 * Copyright 2024 Aspect Build Systems, Inc.
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

package lint

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/golang/mock/gomock"
	. "github.com/onsi/gomega"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"

	"aspect.build/cli/pkg/aspect/root/config"
	rootFlags "aspect.build/cli/pkg/aspect/root/flags"
	bazel_mock "aspect.build/cli/pkg/bazel/mock"
	"aspect.build/cli/pkg/ioutils"
)

const configContents = `configure:
  languages:
    javascript: true
    bzl: true
    go: true
    protobuf: true
  plugins:
    - bazel/terraform/terraform_module.star
    - bazel/configure/sh_library.star
lint:
  aspects:
    - //tools/lint:linters.bzl%%buf
    - //tools/lint:linters.bzl%%eslint
    - //tools/lint:linters.bzl%%vale
    - //tools/lint:linters.bzl%%tfsec

`

func NewTempDir(t *testing.T) string {
	tempDir, err := os.MkdirTemp("", "config_write")
	if err != nil {
		t.Errorf("Failed to create temp directory. %s", err)
		return ""
	}
	t.Cleanup(func() { os.RemoveAll(tempDir) })
	return tempDir
}

func setupVipor(t *testing.T) error {
	tempDir := NewTempDir(t)

	configPath := filepath.Join(tempDir, "config.yaml")
	configContents := []byte(configContents)

	err := os.WriteFile(configPath, configContents, 0644)

	if err != nil {
		return err
	}

	v := viper.GetViper()
	err = config.Load(v, []string{"cmd", "--aspect:config", configPath, "--aspect:nosystem_config", "--aspect:nohome_config"})

	return err
}

func getCMD() *cobra.Command {
	cmd := &cobra.Command{
		Use: "lint",
	}

	cmd.PersistentFlags().Bool(rootFlags.AspectInteractiveFlagName, false, "")

	return cmd
}

func TestLint(t *testing.T) {
	t.Run("do not add -- if it has not been specified", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		err := setupVipor(t)
		g.Expect(err).ToNot(HaveOccurred())

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		bzl := bazel_mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			RunCommand(streams, nil, "build", "--foo", "//...", "--bar", "--aspects=//tools/lint:linters.bzl%%buf,//tools/lint:linters.bzl%%eslint,//tools/lint:linters.bzl%%vale,//tools/lint:linters.bzl%%tfsec", "--output_groups=", "--run_validations=false", "--remote_download_regex='.*AspectRulesLint.*'").
			Return(nil)
		bzl.
			EXPECT().
			IsBazelFlag("build", "remote_download_regex").
			Return(true, nil)

		cmd := getCMD()

		l := &Linter{
			Streams:         streams,
			bzl:             bzl,
			resultsHandlers: make([]LintResultsHandler, 0),
		}

		ctx := context.Background()
		err = l.Run(ctx, cmd, []string{"--foo", "//...", "--bar"})

		g.Expect(err).To(MatchError("BES should always be initiated when running lint"))
	})

	t.Run("accept -- as a first argument", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		err := setupVipor(t)
		g.Expect(err).ToNot(HaveOccurred())

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		bzl := bazel_mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			RunCommand(streams, nil, "build", "--aspects=//tools/lint:linters.bzl%%buf,//tools/lint:linters.bzl%%eslint,//tools/lint:linters.bzl%%vale,//tools/lint:linters.bzl%%tfsec", "--output_groups=", "--run_validations=false", "--remote_download_regex='.*AspectRulesLint.*'", "--", "//...", "-//foo").
			Return(nil)
		bzl.
			EXPECT().
			IsBazelFlag("build", "remote_download_regex").
			Return(true, nil)

		cmd := getCMD()

		l := &Linter{
			Streams:         streams,
			bzl:             bzl,
			resultsHandlers: make([]LintResultsHandler, 0),
		}

		ctx := context.Background()
		err = l.Run(ctx, cmd, []string{"--", "//...", "-//foo"})

		g.Expect(err).To(MatchError("BES should always be initiated when running lint"))
	})

	t.Run("place args before -- before our custom args", func(t *testing.T) {
		g := NewGomegaWithT(t)
		ctrl := gomock.NewController(t)
		defer ctrl.Finish()

		err := setupVipor(t)
		g.Expect(err).ToNot(HaveOccurred())

		var stdout strings.Builder
		streams := ioutils.Streams{Stdout: &stdout}
		bzl := bazel_mock.NewMockBazel(ctrl)
		bzl.
			EXPECT().
			RunCommand(streams, nil, "build", "--some_flag", "--run_validations=true", "--aspects=//tools/lint:linters.bzl%%buf,//tools/lint:linters.bzl%%eslint,//tools/lint:linters.bzl%%vale,//tools/lint:linters.bzl%%tfsec", "--output_groups=", "--run_validations=false", "--remote_download_regex='.*AspectRulesLint.*'", "--", "//...", "-//foo").
			Return(nil)
		bzl.
			EXPECT().
			IsBazelFlag("build", "remote_download_regex").
			Return(true, nil)

		cmd := getCMD()

		l := &Linter{
			Streams:         streams,
			bzl:             bzl,
			resultsHandlers: make([]LintResultsHandler, 0),
		}

		ctx := context.Background()
		err = l.Run(ctx, cmd, []string{"--some_flag", "--run_validations=true", "--", "//...", "-//foo"})

		g.Expect(err).To(MatchError("BES should always be initiated when running lint"))
	})
}
