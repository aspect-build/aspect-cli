/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package query

import (
	"aspect.build/cli/pkg/aspecterrors"
	"aspect.build/cli/pkg/bazel"
	"fmt"
	"github.com/manifoldco/promptui"
	"github.com/pkg/browser"
	"github.com/spf13/cobra"
	"io"
	"io/ioutil"
	"net/url"
	"regexp"
	"strings"

	"aspect.build/cli/pkg/ioutils"
)

type PresetQuery struct {
	Name        string
	Description string
	Query       string
}

type Query struct {
	ioutils.Streams

	bzl           bazel.Bazel
	isInteractive bool

	Presets   []*PresetQuery
	ShowGraph bool
}

func New(streams ioutils.Streams, bzl bazel.Bazel, isInteractive bool) *Query {
	return &Query{
		Streams:       streams,
		bzl:           bzl,
		isInteractive: isInteractive,
	}
}

func (q *Query) Run(_ *cobra.Command, args []string) error {
	presets := make(map[string]*PresetQuery)
	var names []string
	for _, p := range q.Presets {
		presets[p.Name] = p
		names = append(names, fmt.Sprintf("%s: %s", p.Name, p.Description))
	}

	var preset *PresetQuery
	if len(args) == 0 {
		selectQueryPrompt := &promptui.Select{
			Label: "Select a preset query",
			Items: names,
		}

		i, _, err := selectQueryPrompt.Run()
		if err != nil {
			return fmt.Errorf("prompt failed: %w", err)
		}

		preset = q.Presets[i]
	} else {
		maybeQueryOrPreset := args[0]
		if value, ok := presets[maybeQueryOrPreset]; ok {
			// Treat this as the name of the preset query, so don't prompt for it.
			preset = value
		} else {
			// Treat this as a raw query expression
			return q.RunQuery(maybeQueryOrPreset, []string{})
		}
	}

	if preset == nil {
		return fmt.Errorf("unable to determine preset query")
	}

	query := preset.Query
	placeholders := regexp.MustCompile(`(\?[a-zA-Z]*)`).FindAllString(query, -1)

	if len(placeholders) > 0 {
		if (len(placeholders) == len(args) - 1) {
			for i, placeholder := range placeholders {
				query = strings.ReplaceAll(query, placeholder, args[i+1])
			}
		} else {
			for _, placeholder := range placeholders {
				prompt := &promptui.Prompt{
					Label: fmt.Sprintf("Value for '%s'", strings.TrimPrefix(placeholder, "?")),
				}
				val, err := prompt.Run()
	
				if err != nil {
					return fmt.Errorf("prompt failed: %w", err)
				}
	
				query = strings.ReplaceAll(query, placeholder, val)
			}
		}
	}

	return q.RunQuery(query, []string{})
}

func (q *Query) RunQuery(query string, flags []string) error {
	if q.ShowGraph {
		return q.RunQueryAndOpenResult(query, flags)
	}

	bazelCmd := []string{
		"query",
		query,
	}

	bazelCmd = append(bazelCmd, flags...)

	if exitCode, err := q.bzl.Spawn(bazelCmd); exitCode != 0 {
		err = &aspecterrors.ExitError{
			Err:      err,
			ExitCode: exitCode,
		}
		return err
	}

	return nil
}

func (q *Query) RunQueryAndOpenResult(query string, flags []string) error {
	bazelCmd := []string{
		"query",
		query,
		"--output=graph",
	}

	bazelCmd = append(bazelCmd, flags...)

	r, w := io.Pipe()
	bazelErrs := make(chan error, 1)
	defer close(bazelErrs)
	go func() {
		defer w.Close()
		_, err := q.bzl.RunCommand(bazelCmd, w)
		bazelErrs <- err
	}()

	bazelQueryOutput, err := ioutil.ReadAll(r)
	if err != nil {
		return fmt.Errorf("failed to get bazel query response: %w", err)
	}

	if err := <-bazelErrs; err != nil {
		return fmt.Errorf("failed to get bazel query response: %w", err)
	}

	graphVizUrl := fmt.Sprintf("https://edotor.net/?engine=dot#%s", url.PathEscape(string(bazelQueryOutput)))
	if err := browser.OpenURL(graphVizUrl); err != nil {
		return fmt.Errorf("failed to open link in the browser: %w", err)
	}

	return nil
}
