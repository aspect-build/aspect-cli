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

package client

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"hash"
	"io"
	"os"
	"os/exec"
	"strings"

	hclog "github.com/hashicorp/go-hclog"
	goplugin "github.com/hashicorp/go-plugin"

	"aspect.build/cli/pkg/aspect/outputs"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha4/config"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha4/plugin"
	"aspect.build/cli/pkg/plugin/types"
)

// A Factory class for constructing plugin instances.
type Factory interface {
	New(config types.PluginConfig, streams ioutils.Streams) (*PluginInstance, error)
}

func NewFactory() Factory {
	return &clientFactory{}
}

// CustomCommandExecutor requires the Plugin implementations to provide the
// ExecuteCustomCommand method so that the Core can ask over gRPC for a specific command to
// be executed. `cmdName` is the name of the custom command the plugin created.
type CustomCommandExecutor interface {
	ExecuteCustomCommand(cmdName string, ctx context.Context, args []string, bazelStartupArgs []string) error
}

type clientFactory struct {
}

// New calls the goplugin.NewClient with the given config.
func (c *clientFactory) New(aspectplugin types.PluginConfig, streams ioutils.Streams) (*PluginInstance, error) {
	logLevel := hclog.LevelFromString(aspectplugin.LogLevel)
	if logLevel == hclog.NoLevel {
		logLevel = hclog.Warn
	}
	pluginLogger := hclog.New(&hclog.LoggerOptions{
		Name:  aspectplugin.Name,
		Level: logLevel,
	})

	var checksum []byte
	var hash hash.Hash
	if strings.HasPrefix(aspectplugin.From, "//") || strings.HasPrefix(aspectplugin.From, "@") {
		pluginLogger.Info(fmt.Sprintf("building %s plugin from target %s", aspectplugin.Name, aspectplugin.From))

		bzl := bazel.WorkspaceFromWd

		var stderr bytes.Buffer
		buildStreams := ioutils.Streams{
			Stdin:  os.Stdin,
			Stdout: io.Discard,
			Stderr: &stderr,
		}

		// Check `exitCode` before `err` so we can dump the `stderr` when Bazel executed and exited non-zero
		exitCode, err := bzl.RunCommand(buildStreams, nil, "build", aspectplugin.From)
		if exitCode != 0 {
			if exitCode != -1 {
				return nil, fmt.Errorf("failed to build plugin: %w\nstderr:\n%s", err, stderr.String())
			} else {
				return nil, fmt.Errorf("failed to build plugin: %w", err)
			}
		}

		if err != nil {
			// Protect against the case where `err` is set but `exitCode` is 0
			return nil, fmt.Errorf("failed to build plugin: %w", err)
		}

		var stdout bytes.Buffer
		outputsStreams := ioutils.Streams{
			Stdin:  os.Stdin,
			Stdout: &stdout,
			Stderr: io.Discard, // unused
		}
		if err := outputs.New(outputsStreams, bzl).Run(context.Background(), nil, []string{aspectplugin.From, "GoLink"}); err != nil {
			return nil, fmt.Errorf("failed to get plugin path for %q: %w", aspectplugin.From, err)
		}
		aspectplugin.From = strings.TrimSpace(stdout.String())
		checksum = []byte{0}
		hash = noOpHash
	} else {
		if strings.HasPrefix(aspectplugin.From, "github.com/") {
			// Syntax sugar:
			//   from: github.com/org/repo
			// is the same as
			//   from: https://github.com/org/repo/releases/download
			// Example release URL:
			//   https://github.com/aspect-build/aspect-cli-plugin-template/releases/download/v0.1.0/plugin-plugin-linux_amd64
			aspectplugin.From = fmt.Sprintf("https://%s/releases/download", aspectplugin.From)
		}

		if strings.HasPrefix(aspectplugin.From, "http://") || strings.HasPrefix(aspectplugin.From, "https://") {
			// Example release URL:
			//   from:          https://static.aspect.build/aspect
			//   versioned url: https://static.aspect.build/aspect/1.2.3/foo-darwin_amd64
			if len(aspectplugin.Version) < 1 {
				return nil, fmt.Errorf("cannot download plugin %q: the version field is required", aspectplugin.Name)
			}

			pluginLogger.Info(fmt.Sprintf("downloading %s plugin from %s", aspectplugin.Name, aspectplugin.From))

			downloadedPath, err := DownloadPlugin(aspectplugin.From, aspectplugin.Name, aspectplugin.Version)
			if err != nil {
				return nil, err
			}
			aspectplugin.From = downloadedPath
		} else if _, err := os.Stat(aspectplugin.From); err != nil {
			pluginLogger.Warn(fmt.Sprintf("skipping install for plugin: does not exist at path %q.", aspectplugin.From))
			return nil, nil
		}

		checksumFile := fmt.Sprintf("%s.sha256", aspectplugin.From)
		hash = sha256.New()

		if _, err := os.Stat(checksumFile); err != nil {
			// We calculate the hashsum in case it was not provided by the remote server.
			f, err := os.Open(aspectplugin.From)
			if err != nil {
				return nil, fmt.Errorf("failed to calculate hash for %q: %w", aspectplugin.From, err)
			}
			defer f.Close()
			if _, err := io.Copy(hash, f); err != nil {
				return nil, fmt.Errorf("failed to calculate hash for %q: %w", aspectplugin.From, err)
			}
			checksum = hash.Sum(nil)
			hash.Reset()
			if err := os.WriteFile(checksumFile, []byte(hex.EncodeToString(checksum)), 0400); err != nil {
				return nil, fmt.Errorf("failed to calculate hash for %q: %w", aspectplugin.From, err)
			}
		} else {
			b, err := os.ReadFile(checksumFile)
			if err != nil {
				return nil, fmt.Errorf("failed to get hash for %q: %w", aspectplugin.From, err)
			}
			decoded, err := hex.DecodeString(strings.Split(string(b), " ")[0])
			if err != nil {
				return nil, fmt.Errorf("failed to get hash for %q: %w", aspectplugin.From, err)
			}
			checksum = decoded
		}
	}

	pluginLogger.Info(fmt.Sprintf("running %s plugin from %s", aspectplugin.Name, aspectplugin.From))

	secureConfig := &goplugin.SecureConfig{
		Checksum: checksum,
		Hash:     hash,
	}
	clientConfig := &goplugin.ClientConfig{
		HandshakeConfig:  config.Handshake,
		Plugins:          config.PluginMap,
		Cmd:              exec.Command(aspectplugin.From),
		AllowedProtocols: []goplugin.Protocol{goplugin.ProtocolGRPC},
		SyncStdout:       streams.Stdout,
		SyncStderr:       streams.Stderr,
		Logger:           pluginLogger,
		SecureConfig:     secureConfig,
	}

	goclient := goplugin.NewClient(clientConfig)

	rpcClient, err := goclient.Client()
	if err != nil {
		return nil, fmt.Errorf("failed to retrieve plugin client: %w", err)
	}

	rawplugin, err := rpcClient.Dispense(config.DefaultPluginName)
	if err != nil {
		return nil, fmt.Errorf("failed to dispense plugin client: %w", err)
	}

	res := &PluginInstance{
		Plugin:   rawplugin.(plugin.Plugin),
		Provider: goclient,
	}

	if customCommandExecutor, ok := rawplugin.(CustomCommandExecutor); ok {
		res.CustomCommandExecutor = customCommandExecutor
	}

	return res, nil
}

// Provider is an interface for goplugin.Client returned by
// goplugin.NewClient.
type Provider interface {
	Client() (goplugin.ClientProtocol, error)
	Kill()
}

// A PluginInstance consists of the underling Plugin as well
// as any associated objects or metadata.
type PluginInstance struct {
	plugin.Plugin
	Provider
	CustomCommandExecutor
}

// NoOpHash is a hash.Hash that does nothing. It's used for plugins that are
// built from source and required to satisfy the upstream plugin system that
// expects a hash.
type NoOpHash struct{}

var noOpHash hash.Hash = &NoOpHash{}

func (h *NoOpHash) Write(p []byte) (n int, err error) {
	return len(p), nil
}

func (h *NoOpHash) Sum(b []byte) []byte {
	if len(b) == 0 {
		return []byte{0}
	} else {
		b[0] = 0
		return b[:1]
	}
}

func (h *NoOpHash) Reset() {}

func (h *NoOpHash) Size() int {
	return 1
}

func (h *NoOpHash) BlockSize() int {
	return 1
}
