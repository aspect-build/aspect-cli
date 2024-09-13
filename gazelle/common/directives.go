package gazelle

import (
	"log"
	"strings"

	"github.com/bazelbuild/bazel-gazelle/rule"
)

const (
	// Directive_GenerationMode represents the directive that controls the BUILD generation
	// mode. See below for the GenerationModeType constants.
	Directive_GenerationMode = "generation_mode"
)

// GenerationModeType represents one of the generation modes.
type GenerationModeType string

// Generation modes
const (
	// Update: update and maintain existing BUILD files
	GenerationModeUpdate GenerationModeType = "update"

	// Create: create new and updating existing BUILD files
	GenerationModeCreate GenerationModeType = "create"
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
