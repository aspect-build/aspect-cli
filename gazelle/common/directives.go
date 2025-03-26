package gazelle

import (
	"log"
	"strings"

	"github.com/bazelbuild/bazel-gazelle/rule"
)

func ReadEnabled(d rule.Directive) bool {
	switch strings.TrimSpace(d.Value) {
	case "enabled":
		return true
	case "disabled":
		return false
	default:
		log.Fatalf("Invalid directive %s enabled/disabled value: %s", d.Key, d.Value)
		return false
	}
}
