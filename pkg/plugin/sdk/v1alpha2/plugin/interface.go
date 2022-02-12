/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package plugin

import (
	buildeventstream "aspect.build/cli/bazel/buildeventstream/proto"
	"aspect.build/cli/pkg/ioutils"
)

// Plugin determines how an aspect Plugin should be implemented.
type Plugin interface {
	BEPEventCallback(event *buildeventstream.BuildEvent) error
	Setup(properties []byte) error
	PostBuildHook(
		isInteractiveMode bool,
		promptRunner ioutils.PromptRunner,
	) error
	PostTestHook(
		isInteractiveMode bool,
		promptRunner ioutils.PromptRunner,
	) error
	PostRunHook(
		isInteractiveMode bool,
		promptRunner ioutils.PromptRunner,
	) error
}

// Base satisfies the Plugin interface. For plugins that only implement a subset
// of the Plugin interface, using this as a base will give the advantage of not
// needing to implement the empty methods.
type Base struct{}

var _ Plugin = (*Base)(nil)

// Setup satisfies Plugin.Setup.
func (*Base) Setup([]byte) error {
	return nil
}

// BEPEventCallback satisfies Plugin.BEPEventCallback.
func (*Base) BEPEventCallback(*buildeventstream.BuildEvent) error {
	return nil
}

// PostBuildHook satisfies Plugin.PostBuildHook.
func (*Base) PostBuildHook(bool, ioutils.PromptRunner) error {
	return nil
}

// PostTestHook satisfies Plugin.PostTestHook.
func (*Base) PostTestHook(bool, ioutils.PromptRunner) error {
	return nil
}

// PostRunHook satisfies Plugin.PostRunHook.
func (*Base) PostRunHook(bool, ioutils.PromptRunner) error {
	return nil
}
