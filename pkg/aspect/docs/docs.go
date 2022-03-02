/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package docs

import (
	"fmt"
	"os"
	"strings"

	"aspect.build/cli/pkg/ioutils"
	"github.com/pkg/browser"
	"github.com/spf13/cobra"
)

type Docs struct {
	ioutils.Streams
}

func New(streams ioutils.Streams) *Docs {
	return &Docs{
		Streams: streams,
	}
}

func (v *Docs) Run(_ *cobra.Command, args []string) error {
	// TODO: we should open the browser to the bazel version matching what is running
	dest := "https://bazel.build/docs"

	// Detect requests for docs on rules, which we host
	if len(args) == 1 {
		if strings.HasPrefix(args[0], "rules_") {
			dest = fmt.Sprintf("https://docs.aspect.build/%s", args[0])
		} else {
			dest = fmt.Sprintf("https://bazel.build/reference/%s.html", args[0])
		}
	}
	// TODO: a way to lookup whatever the user typed after "docs" using docs.aspect.build search
	// as far as I can tell, Algolia doesn't provide a way to render results on a dedicated search page
	// so I can't find a way to hyperlink to a search result.
	if err := browser.OpenURL(dest); err != nil {
		fmt.Fprintf(os.Stderr, "Failed to open link in the browser: %v\n", err)
	}

	return nil
}
