package buildinfo

import (
	"fmt"
)

type VersionFormat int

const (
	ConventionalFormat VersionFormat = iota
	GNUFormat
)

func UtilityVersion(name string, version string, format VersionFormat) string {
	switch format {
	case GNUFormat:
		return fmt.Sprintf("%s %s", name, version)
	case ConventionalFormat:
		// Conventional is the default case
		fallthrough
	default:
		// Use the Conventional format, if not recognized
		return fmt.Sprintf("%s version: %s", name, version)
	}
}
