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

package config_test

import (
	"fmt"
	"os"
	"path"
	"path/filepath"
	"testing"

	"github.com/aspect-build/aspect-cli/pkg/aspect/root/config"
	. "github.com/onsi/gomega"
	"github.com/spf13/viper"
)

const (
	configDirectory = ".aspect/cli"
	configBasename  = ".aspect/cli/config"
	configType      = "yaml"
	configFilename  = ".aspect/cli/config.yaml"
)

func NewTempDir(t *testing.T) string {
	tempDir, err := os.MkdirTemp("", "config_write")
	if err != nil {
		t.Errorf("Failed to create temp directory. %s", err)
		return ""
	}
	t.Cleanup(func() { os.RemoveAll(tempDir) })
	return tempDir
}

func NewViper(tempDir string) *viper.Viper {
	v := viper.New()
	v.AddConfigPath(tempDir)
	v.SetConfigName(configBasename)
	v.SetConfigType(configType)
	return v
}

func TestWrite(t *testing.T) {
	g := NewWithT(t)
	tempDir := NewTempDir(t)
	configPath := filepath.Join(tempDir, configFilename)

	v := NewViper(tempDir)

	// Create the config directory exists under the tempDir directory
	os.MkdirAll(path.Join(tempDir, configDirectory), os.ModePerm)

	// Set a value
	key := "chicken"
	value := "hello"
	v.Set(key, value)

	// Verify initial write succeeds
	err := config.Write(v)
	g.Expect(err).ToNot(HaveOccurred())
	g.Expect(configPath).To(BeAnExistingFile())

	// Verify value was written
	v = NewViper(tempDir)
	err = v.ReadInConfig()
	g.Expect(err).ToNot(HaveOccurred())
	g.Expect(v.Get(key)).To(Equal(value))

	// Set a new value
	newValue := "goodbye"
	v.Set(key, newValue)

	// Verify second write succeeds
	err = config.Write(v)
	g.Expect(err).ToNot(HaveOccurred())
	g.Expect(configPath).To(BeAnExistingFile())

	// Verify new value was written
	v = NewViper(tempDir)
	err = v.ReadInConfig()
	g.Expect(err).ToNot(HaveOccurred())
	g.Expect(v.Get(key)).To(Equal(newValue))
}

func TestLoad(t *testing.T) {
	g := NewWithT(t)
	tempDir := NewTempDir(t)

	workspaceFilePath := filepath.Join(tempDir, "WORKSPACE")
	workspaceConfigPath := filepath.Join(tempDir, configFilename)
	userConfigPath := filepath.Join(tempDir, "myconfig.yaml")

	// Create the config directory exists under the tempDir directory
	os.MkdirAll(path.Join(tempDir, configDirectory), os.ModePerm)

	err := os.WriteFile(workspaceFilePath, []byte{}, 0644)
	g.Expect(err).ToNot(HaveOccurred())

	workspaceConfigContents := []byte(`configure:
  languages:
    javascript: true
    go: true
    protobuf: true
something: from_workspace_config
plugins:
  - name: foo
    from: https://static.plugins.com/foo
    version: 1.2.3
  - name: fum
    from: https://static.plugins.com/fum
    version: 1.2.3
`)

	err = os.WriteFile(workspaceConfigPath, workspaceConfigContents, 0644)
	g.Expect(err).ToNot(HaveOccurred())

	userConfigContents := []byte(`configure:
  languages:
    javascript: true
    go: false
    protobuf: false
something_else: from_myconfig
plugins:
  - name: bar
    from: https://static.plugins.com/bar
    version: 1.2.3
  - name: foo
    from: https://static.plugins.com/foo
    version: 3.2.1
    log_level: debug
`)

	err = os.WriteFile(userConfigPath, userConfigContents, 0644)
	g.Expect(err).ToNot(HaveOccurred())

	// Config file loader searches the CWD for the WORKSPACE file
	os.Chdir(tempDir)

	v := viper.New()

	err = config.Load(v, []string{"cmd", "--aspect:config", "myconfig.yaml", "--aspect:nosystem_config", "--aspect:nohome_config"})
	g.Expect(err).ToNot(HaveOccurred())

	g.Expect(v.Get("something")).To(Equal("from_workspace_config"))
	g.Expect(v.Get("something_else")).To(Equal("from_myconfig"))

	// User config "configure" should override the workspace config "configure"
	g.Expect(fmt.Sprintf("%v", v.Get("configure"))).To(Equal("map[languages:map[go:false javascript:true protobuf:false]]"))

	// Plugin lists should be merged with plugins that have the same name being overrides
	g.Expect(fmt.Sprintf("%v", v.Get("plugins"))).To(Equal("[map[from:https://static.plugins.com/foo log_level:debug name:foo version:3.2.1] map[from:https://static.plugins.com/fum name:fum version:1.2.3] map[from:https://static.plugins.com/bar name:bar version:1.2.3]]"))
}
