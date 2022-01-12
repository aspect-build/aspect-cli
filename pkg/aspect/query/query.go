/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package query

import (
	"fmt"
	"regexp"
	"strings"

	"github.com/manifoldco/promptui"
	"github.com/spf13/cobra"

	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

type PresetQuery struct {
	Name        string
	Description string
	Query       string
}

type Query struct {
	ioutils.Streams

	Bzl           bazel.Bazel
	IsInteractive bool

	Presets []*PresetQuery

	GetAPrompt func(label string) PromptRunner
	GetASelect func(presetNames []string) SelectRunner
}

func New(streams ioutils.Streams, bzl bazel.Bazel, isInteractive bool) *Query {
	// TODO: Queries should be loadable from the plugin config
	// https://github.com/aspect-build/aspect-cli/issues/98
	presets := []*PresetQuery{
		{
			Name:        "why",
			Description: "Determine why targetA depends on targetB",
			Query:       "somepath(?targetA, ?targetB)",
		},
	}

	return &Query{
		Streams:       streams,
		Bzl:           bzl,
		IsInteractive: isInteractive,
		Presets:       presets,
		GetAPrompt:    GetAPrompt,
		GetASelect:    GetASelect,
	}
}

func (q *Query) Run(cmd *cobra.Command, args []string) error {
	presets := make(map[string]*PresetQuery)
	presetNames := make([]string, len(q.Presets))
	for i, p := range q.Presets {
		if _, exists := presets[p.Name]; exists {
			err := fmt.Errorf("duplicated preset query name %q", p.Name)
			return fmt.Errorf("failed to run 'aspect %s': %w", cmd.Use, err)
		}
		presets[p.Name] = p
		presetNames[i] = fmt.Sprintf("%s: %s", p.Name, p.Description)
	}

	var preset *PresetQuery
	if len(args) == 0 {
		selectQueryPrompt := q.GetASelect(presetNames)

		i, _, err := selectQueryPrompt.Run()

		if err != nil {
			return fmt.Errorf("prompt failed: %w", err)
		}

		preset = q.Presets[i]
	} else {
		maybeQueryOrPreset := args[0]
		if value, ok := presets[maybeQueryOrPreset]; ok {
			// Treat this as the name of the preset query, so don't prompt for it.
			fmt.Fprintf(q.Streams.Stdout, "Preset query \"%s\" selected\n", value.Name)
			fmt.Fprintf(q.Streams.Stdout, "%s: %s\n", value.Name, value.Description)
			preset = value
		} else {
			// Treat this as a raw query expression.
			return q.RunQuery(maybeQueryOrPreset)
		}
	}

	if preset == nil {
		return fmt.Errorf("unable to determine preset query")
	}

	query := preset.Query
	placeholders := regexp.MustCompile(`(\?[a-zA-Z]*)`).FindAllString(query, -1)

	if len(placeholders) == len(args)-1 {
		for i, placeholder := range placeholders {
			query = strings.ReplaceAll(query, placeholder, args[i+1])
		}
	} else if len(placeholders) > 0 {
		for _, placeholder := range placeholders {
			label := fmt.Sprintf("Value for '%s'", strings.TrimPrefix(placeholder, "?"))
			prompt := q.GetAPrompt(label)
			val, err := prompt.Run()

			if err != nil {
				return fmt.Errorf("prompt failed: %w", err)
			}

			query = strings.ReplaceAll(query, placeholder, val)
		}
	}

	return q.RunQuery(query)
}

type PromptRunner interface {
	Run() (string, error)
}

func GetAPrompt(label string) PromptRunner {
	return &promptui.Prompt{
		Label: label,
	}
}

type SelectRunner interface {
	Run() (int, string, error)
}

func GetASelect(presetNames []string) SelectRunner {
	return &promptui.Select{
		Label: "Select a preset query",
		Items: presetNames,
	}
}

func (q *Query) RunQuery(query string) error {
	bazelCmd := []string{
		"query",
		query,
	}

	bazelCmd = append(bazelCmd)

	if exitCode, err := q.Bzl.Spawn(bazelCmd); exitCode != 0 {
		err = &aspecterrors.ExitError{
			Err:      err,
			ExitCode: exitCode,
		}
		return fmt.Errorf("failed to run query %q: %w", query, err)
	}

	return nil
}
