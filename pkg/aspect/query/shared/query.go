/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package shared

/*
This package is meant to contain code that will be shared between the 3 query verbs. query, aquery and cquery
*/

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

var placeholderRegex = regexp.MustCompile(`(\?[a-zA-Z]*)`)

type PresetQuery struct {
	Name        string
	Description string
	Query       string
	Verb        string
}

type PromptRunner interface {
	Run() (string, error)
}

func Prompt(label string) PromptRunner {
	return &promptui.Prompt{
		Label: label,
	}
}

type ConfirmationRunner interface {
	Run() (string, error)
}

func Confirmation(question string) ConfirmationRunner {
	return &promptui.Prompt{
		Label:     question,
		IsConfirm: true,
	}
}

type SelectRunner interface {
	Run() (int, string, error)
}

func Select(presetNames []string) SelectRunner {
	return &promptui.Select{
		Label: "Select a preset query",
		Items: presetNames,
	}
}

func PrecannedQueries(verb string) []*PresetQuery {
	// TODO: Queries should be loadable from the plugin config
	// https://github.com/aspect-build/aspect-cli/issues/98
	presets := []*PresetQuery{
		{
			Name:        "why",
			Description: "Determine why targetA depends on targetB",
			Query:       "somepath(?targetA, ?targetB)",
			Verb:        "query",
		},
		{
			Name:        "deps",
			Description: "Get the deps of a target",
			Query:       "deps(?target)",
			Verb:        "query",
		},
		{
			Name:        "adeps",
			Description: "Get the deps of a target",
			Query:       "deps(?target)",
			Verb:        "aquery",
		},
		{
			Name:        "cdeps",
			Description: "Get the deps of a target",
			Query:       "deps(?target)",
			Verb:        "cquery",
		},
	}

	switch verb {
	case "query":
		return filterPrecannedQueries("query", presets)
	case "cquery":
		return filterPrecannedQueries("cquery", presets)
	case "aquery":
		return filterPrecannedQueries("aquery", presets)
	}

	return presets
}

func filterPrecannedQueries(verb string, presets []*PresetQuery) []*PresetQuery {
	filteredPresets := []*PresetQuery{}
	for _, presetQuery := range presets {

		if presetQuery.Verb == verb {
			filteredPresets = append(filteredPresets, presetQuery)
		}
	}

	return filteredPresets
}

func ProcessQueries(presets []*PresetQuery) (map[string]*PresetQuery, []string, error) {
	processedPresets := make(map[string]*PresetQuery)
	presetNames := make([]string, len(presets))
	for i, p := range presets {
		if _, exists := processedPresets[p.Name]; exists {
			err := fmt.Errorf("duplicated preset query name %q", p.Name)
			return processedPresets, presetNames, err
		}
		processedPresets[p.Name] = p
		presetNames[i] = fmt.Sprintf("%s: %s", p.Name, p.Description)
	}

	return processedPresets, presetNames, nil
}

func RunQuery(bzl bazel.Bazel, verb string, query string) error {
	bazelCmd := []string{
		verb,
		query,
	}

	if exitCode, err := bzl.Spawn(bazelCmd); exitCode != 0 {
		err = &aspecterrors.ExitError{
			Err:      err,
			ExitCode: exitCode,
		}
		return fmt.Errorf("failed to run %q %q: %w", verb, query, err)
	}

	return nil
}

func ReplacePlaceholders(query string, args []string, p func(label string) PromptRunner) (string, error) {
	placeholders := placeholderRegex.FindAllString(query, -1)

	if len(placeholders) == len(args)-1 {
		for i, placeholder := range placeholders {
			fmt.Printf("%s set to %s\n", strings.Replace(placeholder, "?", "", 1), args[i+1])
			// todo.... Print out targetA was set to //foo and targetB was set to //bar
			query = strings.ReplaceAll(query, placeholder, args[i+1])
		}
	} else if len(placeholders) > 0 {
		for _, placeholder := range placeholders {
			label := fmt.Sprintf("Value for '%s'", strings.TrimPrefix(placeholder, "?"))
			prompt := p(label)
			val, err := prompt.Run()

			if err != nil {
				return "", err
			}

			query = strings.ReplaceAll(query, placeholder, val)
		}
	}

	return query, nil
}

func SelectQuery(
	verb string,
	processedPresets map[string]*PresetQuery,
	rawPresets []*PresetQuery,
	presetNames []string,
	streams ioutils.Streams,
	args []string,
	s func(presetNames []string) SelectRunner,
) (string, string, bool, error) {

	var preset *PresetQuery
	if len(args) == 0 {
		selectQueryPrompt := s(presetNames)

		i, _, err := selectQueryPrompt.Run()

		if err != nil {
			return verb, "", false, err
		}

		preset = rawPresets[i]
	} else {
		maybeQueryOrPreset := args[0]
		if value, ok := processedPresets[maybeQueryOrPreset]; ok {
			// Treat this as the name of the preset query, so don't prompt for it.
			fmt.Fprintf(streams.Stdout, "Preset query \"%s\" selected\n", value.Name)
			fmt.Fprintf(streams.Stdout, "%s: %s\n", value.Name, value.Description)
			preset = value
		} else {
			// Treat this as a raw query expression.
			return verb, maybeQueryOrPreset, false, nil
		}
	}

	if preset == nil {
		err := fmt.Errorf("unable to determine preset query")
		return verb, "", false, err
	}

	return preset.Verb, preset.Query, true, nil
}

func GetPrettyError(cmd *cobra.Command, err error) error {
	return fmt.Errorf("failed to run 'aspect %s': %w", cmd.Use, err)
}
