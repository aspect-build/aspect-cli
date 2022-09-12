package versionwriter

import (
	"fmt"
	"io"
	"strings"

	"aspect.build/cli/buildinfo"
)

type Format int

const (
	GNU Format = iota
	Conventional
)

type VersionWriter struct {
	Name    string
	Version string
	Format  Format
}

func New(name string, version string, format Format) *VersionWriter {
	return &VersionWriter{
		Name:    name,
		Version: version,
		Format:  format,
	}
}

func NewFromBuildInfo(name string, format Format) *VersionWriter {
	var versionBuilder strings.Builder
	if buildinfo.HasRelease() {
		versionBuilder.WriteString(buildinfo.Release)
		if !buildinfo.IsClean() {
			versionBuilder.WriteString(" (with local changes)")
		}
	} else {
		versionBuilder.WriteString("unknown [not built with --stamp]")
	}
	return New(name, versionBuilder.String(), format)
}

func (vw VersionWriter) String() string {
	switch vw.Format {
	case GNU:
		return fmt.Sprintf("%s %s\n", vw.Name, vw.Version)
	case Conventional:
		return fmt.Sprintf("%s version: %s\n", vw.Name, vw.Version)
	default:
		// Use the Conventional format, if not recognized
		return fmt.Sprintf("%s version: %s\n", vw.Name, vw.Version)
	}
}

func (vw VersionWriter) Print(w io.Writer) (n int, err error) {
	return fmt.Fprintf(w, vw.String())
}
