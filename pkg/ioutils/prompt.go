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
