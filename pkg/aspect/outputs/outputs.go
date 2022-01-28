/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package outputs

import (
	"fmt"
	"log"
	"strings"

	"aspect.build/cli/pkg/bazel"
	"aspect.build/cli/pkg/ioutils"
)

type Outputs struct {
	ioutils.Streams
	Bzl           bazel.Bazel
	IsInteractive bool
}

func New(streams ioutils.Streams, bzl bazel.Bazel, isInteractive bool) *Outputs {
	return &Outputs{
		Streams:       streams,
		Bzl:           bzl,
		IsInteractive: isInteractive,
	}
}

func (q *Outputs) Run(args []string, bzl bazel.Bazel) error {
	if len(args) < 1 {
		log.Fatalf("TODO: interactive ask for the label")
	}
	query := args[0]
	var mnemonicFilter string
	if len(args) > 1 {
		mnemonicFilter = args[1]
	} else {
		mnemonicFilter = ""
	}
	agc, err := bzl.AQuery(query)
	if err != nil {
		return err
	}

	// Use RAM to store lookup maps for these identifiers
	// rather than an O(n^2) algorithm of searching on each access
	frags := make(map[uint32]*bazel.PathFragment)
	for _, f := range agc.PathFragments {
		frags[f.Id] = f
	}
	arts := make(map[uint32]*bazel.Artifact)
	for _, a := range agc.Artifacts {
		arts[a.Id] = a
	}

	// The paths in the proto data are organized as a trie
	// to make the representation more compact
	// https://en.wikipedia.org/wiki/Trie
	// Make a map to store each prefix so we can memoize common paths
	prefixes := make(map[uint32]*[]string)

	// Declare a recursive function to walk up the trie to the root
	var prefix func(pathID uint32) []string

	prefix = func(pathID uint32) []string {
		if prefixes[pathID] != nil {
			return *prefixes[pathID]
		}
		fragment := frags[pathID]
		// reconstruct the path from the parent pointers
		segments := []string{fragment.Label}

		if fragment.ParentId > 0 {
			segments = append(segments, prefix(fragment.ParentId)...)
		}
		prefixes[pathID] = &segments
		return segments
	}

	for _, a := range agc.Actions {
		if len(mnemonicFilter) > 0 && (a.Mnemonic != mnemonicFilter) {
			continue
		}
		for _, i := range a.OutputIds {
			artifact := arts[i]
			segments := prefix(artifact.PathFragmentId)
			var path strings.Builder
			// assemble in reverse order
			for i := len(segments) - 1; i >= 0; i -= 1 {
				path.WriteString(segments[i])
				if i > 0 {
					path.WriteString("/")
				}
			}
			fmt.Printf("%s %s\n", a.Mnemonic, path.String())
		}
	}
	return nil
}
