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

package configutils_test

import (
	"os"
	"path/filepath"
	"testing"

	"aspect.build/cli/pkg/configutils"
	. "github.com/onsi/gomega"
	"github.com/spf13/viper"
)

const (
	configBasename = ".aspect"
	configType     = "yaml"
	configFilename = ".aspect.yaml"
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
	v.SetConfigName(".aspect")
	v.SetConfigType("yaml")
	return v
}

func TestWrite(t *testing.T) {
	g := NewWithT(t)
	tempDir := NewTempDir(t)
	configPath := filepath.Join(tempDir, configFilename)

	v := NewViper(tempDir)

	// Set a value
	key := "chicken"
	value := "hello"
	v.Set(key, value)

	// Verify initial write succeeds
	err := configutils.Write(v)
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
	err = configutils.Write(v)
	g.Expect(err).ToNot(HaveOccurred())
	g.Expect(configPath).To(BeAnExistingFile())

	// Verify new value was written
	v = NewViper(tempDir)
	err = v.ReadInConfig()
	g.Expect(err).ToNot(HaveOccurred())
	g.Expect(v.Get(key)).To(Equal(newValue))
}
