/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package main

import (
	"fmt"
	"regexp"
	"strings"
	"sync"

	goplugin "github.com/hashicorp/go-plugin"
	"gopkg.in/yaml.v2"

	buildeventstream "aspect.build/cli/bazel/buildeventstream/proto"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/sdk/v1alpha2/config"
)

func main() {
	goplugin.Serve(config.NewConfigFor(NewDefaultPlugin()))
}

type ErrorAugmentorPlugin struct {
	properties             []byte
	hintMap                map[*regexp.Regexp]string
	yamlUnmarshalStrict    func(in []byte, out interface{}) (err error)
	helpfulHints           *helpfulHintSet
	helpfulHintsMutex      sync.Mutex
	errorMessages          chan string
	errorMessagesWaitGroup sync.WaitGroup
}

func NewDefaultPlugin() *ErrorAugmentorPlugin {
	return NewPlugin()
}

func NewPlugin() *ErrorAugmentorPlugin {
	return &ErrorAugmentorPlugin{
		properties:          nil,
		hintMap:             map[*regexp.Regexp]string{},
		yamlUnmarshalStrict: yaml.UnmarshalStrict,
		helpfulHints:        &helpfulHintSet{nodes: make(map[helpfulHintNode]struct{})},
	}
}

// pluginProperties is used to unmarshal the error mappings and processor_count from the plugin
// properties provided by the CLI core.
type pluginProperties struct {
	ErrorMappings  map[string]string `yaml:"error_mappings"`
	ProcessorCount int               `yaml:"processor_count"`
}

func (plugin *ErrorAugmentorPlugin) Setup(
	properties []byte,
) error {
	plugin.properties = properties

	var processedProperties pluginProperties
	if err := plugin.yamlUnmarshalStrict(properties, &processedProperties); err != nil {
		return fmt.Errorf("failed to setup: failed to parse properties: %w", err)
	}

	// change map keys into regex objects now so they are ready to use and we only need to compile the regex once
	for pattern, message := range processedProperties.ErrorMappings {
		regex, err := regexp.Compile(pattern)
		if err != nil {
			return err
		}

		plugin.hintMap[regex] = message
	}

	plugin.errorMessages = make(chan string, processedProperties.ProcessorCount*processedProperties.ProcessorCount)
	for i := 0; i < processedProperties.ProcessorCount; i++ {
		go plugin.errorMessageProcessor()
		plugin.errorMessagesWaitGroup.Add(1)
	}

	return nil
}

func (plugin *ErrorAugmentorPlugin) BEPEventCallback(event *buildeventstream.BuildEvent) error {
	aborted := event.GetAborted()
	if aborted != nil {
		plugin.errorMessages <- aborted.Description

		// We exit early here because there will not be a progress message when the event was of type "aborted".
		return nil
	}

	progress := event.GetProgress()

	if progress != nil {
		stdout := progress.GetStdout()
		if stdout != "" {
			plugin.errorMessages <- stdout
		}

		stderr := progress.GetStderr()
		if stderr != "" {
			plugin.errorMessages <- stderr
		}
	}

	return nil
}

func (plugin *ErrorAugmentorPlugin) errorMessageProcessor() {
	for errorMessage := range plugin.errorMessages {
		plugin.processErrorMessage(errorMessage)
	}
	plugin.errorMessagesWaitGroup.Done()
}

func (plugin *ErrorAugmentorPlugin) processErrorMessage(errorMessage string) {
	for regex, helpfulHint := range plugin.hintMap {
		matches := regex.FindStringSubmatch(errorMessage)

		if len(matches) > 0 {

			// apply regex capture group replacements to given hint
			for i, match := range matches {
				if i == 0 {
					// skipping the first match because it will always contain the entire result
					// of the regex match. We are only after specific capture groups
					continue
				}
				helpfulHint = strings.ReplaceAll(helpfulHint, fmt.Sprint("$", i), match)
			}

			plugin.helpfulHintsMutex.Lock()
			plugin.helpfulHints.insert(helpfulHint)
			plugin.helpfulHintsMutex.Unlock()
		}
	}
}

func (plugin *ErrorAugmentorPlugin) PostBuildHook(
	isInteractiveMode bool,
	promptRunner ioutils.PromptRunner,
) error {
	close(plugin.errorMessages)
	plugin.errorMessagesWaitGroup.Wait()

	if plugin.helpfulHints.size == 0 {
		return nil
	}

	plugin.printBreak()

	plugin.printMiddle("Aspect CLI Error Augmentor")
	plugin.printMiddle("")

	for node := plugin.helpfulHints.head; node != nil; node = node.next {
		plugin.printMiddle("- " + node.helpfulHint)
	}

	plugin.printBreak()
	return nil
}

func (plugin *ErrorAugmentorPlugin) printBreak() {
	// using buffer so that we can easily determine the current length of the string and
	// ensure we create a proper square with a straight border
	var b strings.Builder

	fmt.Fprintf(&b, " ")

	for i := 0; i < 90; i++ {
		fmt.Fprintf(&b, "-")
	}

	fmt.Fprintf(&b, " ")

	fmt.Println(b.String())
}

func (plugin *ErrorAugmentorPlugin) printMiddle(str string) {
	// using buffer so that we can easily determine the current length of the string and
	// ensure we create a proper square with a straight border
	var b strings.Builder

	fmt.Fprintf(&b, "| ")
	fmt.Fprintf(&b, str)

	for b.Len() < 91 {
		fmt.Fprintf(&b, " ")
	}

	fmt.Fprintf(&b, "|")
	fmt.Println(b.String())
}

func (plugin *ErrorAugmentorPlugin) PostTestHook(
	isInteractiveMode bool,
	promptRunner ioutils.PromptRunner,
) error {
	return plugin.PostBuildHook(isInteractiveMode, promptRunner)
}

func (plugin *ErrorAugmentorPlugin) PostRunHook(
	isInteractiveMode bool,
	promptRunner ioutils.PromptRunner,
) error {
	return plugin.PostBuildHook(isInteractiveMode, promptRunner)
}

type helpfulHintSet struct {
	head  *helpfulHintNode
	tail  *helpfulHintNode
	nodes map[helpfulHintNode]struct{}
	size  int
}

func (s *helpfulHintSet) insert(helpfulHint string) {
	node := helpfulHintNode{
		helpfulHint: helpfulHint,
	}
	if _, exists := s.nodes[node]; !exists {
		s.nodes[node] = struct{}{}
		if s.head == nil {
			s.head = &node
		} else {
			s.tail.next = &node
		}
		s.tail = &node
		s.size++
	}
}

type helpfulHintNode struct {
	next        *helpfulHintNode
	helpfulHint string
}
