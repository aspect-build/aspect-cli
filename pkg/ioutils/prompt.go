/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
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
