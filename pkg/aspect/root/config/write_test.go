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
	"path/filepath"
	"testing"

	"aspect.build/cli/pkg/aspect/root/config"
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
	os.MkdirAll(fmt.Sprintf("%s/%s", tempDir, configDirectory), os.ModePerm)

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
