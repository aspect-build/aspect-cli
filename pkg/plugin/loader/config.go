/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package loader

// SetupConfig represents a plugin configuration parsed from the aspectplugins
// file.
type SetupConfig struct {
	File       *AspectPluginFile
	Properties []byte
}

// NewSetupConfig creates a new SetupConfig.
func NewSetupConfig(
	file *AspectPluginFile,
	properties []byte,
) *SetupConfig {
	return &SetupConfig{
		File:       file,
		Properties: properties,
	}
}

// AspectPluginFile contains metadata for the aspectplugins file relevant for
// a plugin.
type AspectPluginFile struct {
	Path string
}

// NewAspectPluginFile creates a new AspectPluginFile.
func NewAspectPluginFile(path string) *AspectPluginFile {
	return &AspectPluginFile{
		Path: path,
	}
}
