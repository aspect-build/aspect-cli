package gazelle

import (
	plugin "github.com/aspect-build/aspect-cli/gazelle/host/plugin"
)

var builtinKinds = []plugin.RuleKind{
	// Native
	plugin.RuleKind{
		Name: "filegroup",
		KindInfo: plugin.KindInfo{
			NonEmptyAttrs:  []string{"srcs"},
			MergeableAttrs: []string{"srcs"},
		},
	},

	// @aspect_bazel_lib
	plugin.RuleKind{
		Name: "copy_to_bin",
		From: "@aspect_bazel_lib//lib:copy_to_bin.bzl",
		KindInfo: plugin.KindInfo{
			NonEmptyAttrs:  []string{"srcs"},
			MergeableAttrs: []string{"srcs"},
		},
	},
}
