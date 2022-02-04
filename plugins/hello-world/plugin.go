/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package main

import (
	"context"
	"fmt"

	goplugin "github.com/hashicorp/go-plugin"

	buildeventstream "aspect.build/cli/bazel/buildeventstream/proto"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha2/config"
	aspectplugin "aspect.build/cli/pkg/plugin/sdk/v1alpha2/plugin"
)

func main() {
	goplugin.Serve(config.NewConfigFor(NewDefaultPlugin()))
}

type HelloWorldPlugin struct{}

func NewDefaultPlugin() *HelloWorldPlugin {
	return NewPlugin()
}

func NewPlugin() *HelloWorldPlugin {
	return &HelloWorldPlugin{}
}

func (plugin *HelloWorldPlugin) BEPEventCallback(event *buildeventstream.BuildEvent) error {
	return nil
}

func (plugin *HelloWorldPlugin) PostBuildHook(
	isInteractiveMode bool,
	promptRunner ioutils.PromptRunner,
) error {
	return nil
}

func (plugin *HelloWorldPlugin) PostTestHook(
	isInteractiveMode bool,
	promptRunner ioutils.PromptRunner,
) error {
	return plugin.PostBuildHook(isInteractiveMode, promptRunner)
}

func (plugin *HelloWorldPlugin) PostRunHook(
	isInteractiveMode bool,
	promptRunner ioutils.PromptRunner,
) error {
	return plugin.PostBuildHook(isInteractiveMode, promptRunner)
}

func (plugin *HelloWorldPlugin) CustomCommands() ([]*aspectplugin.Command, error) {
	commandList := make([]*aspectplugin.Command, 0)

	commandList = append(commandList, &aspectplugin.Command{
		Use:       "hello-world",
		ShortDesc: "Print 'Hello World!' to the command line.",
		LongDesc:  "Print 'Hello World!' to the command line. Echo any given argument. Then run a 'bazel help'",
		Run: func(ctx context.Context, args []string) error {
			fmt.Println("Hello World!")
			fmt.Print("Arguments passed to command: ")
			fmt.Println(args)
			fmt.Println("Going to run: 'bazel help'")

			bzl := aspectplugin.GetBazel(ctx)

			bzl.Spawn([]string{"help"})

			return nil
		},
	})

	return commandList, nil
}
