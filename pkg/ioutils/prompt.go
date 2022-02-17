/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package ioutils

import "github.com/manifoldco/promptui"

// PromptRunner is the interface that wraps the promptui.Prompt and makes a call
// to it from the aspect CLI Core.
type PromptRunner interface {
	Run(prompt promptui.Prompt) (string, error)
}

// promptRunner implements a default PromptRunner.
type promptRunner struct{}

// NewPromptRunner creates a new default prompt runner.
func NewPromptRunner() PromptRunner {
	return &promptRunner{}
}

// Run runs the given prompt.
func (pr *promptRunner) Run(prompt promptui.Prompt) (string, error) {
	return prompt.Run()
}
