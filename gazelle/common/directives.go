package gazelle

const (
	// Directive_GenerationMode represents the directive that controls the BUILD generation
	// mode. See below for the GenerationModeType constants.
	Directive_GenerationMode = "generation_mode"
)

// GenerationModeType represents one of the generation modes.
type GenerationModeType string

// Generation modes
const (
	// None: do not update or create any BUILD files
	GenerationModeNone GenerationModeType = "none"

	// Update: update and maintain existing BUILD files
	GenerationModeUpdate GenerationModeType = "update"

	// Create: create new and updating existing BUILD files
	GenerationModeCreate GenerationModeType = "create"
)
