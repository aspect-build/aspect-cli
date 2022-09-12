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

	NotCleanVersionSuffix = " (with local changes)"
	NoReleaseVersion      = "unknown [not built with --stamp]"
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

func NewFromBuildInfo(name string, bi buildinfo.BuildInfo, format Format) *VersionWriter {
	var versionBuilder strings.Builder
	if bi.HasRelease() {
		versionBuilder.WriteString(bi.Release)
		if !bi.IsClean() {
			versionBuilder.WriteString(NotCleanVersionSuffix)
		}
	} else {
		versionBuilder.WriteString(NoReleaseVersion)
	}
	return New(name, versionBuilder.String(), format)
}

func (vw VersionWriter) String() string {
	switch vw.Format {
	case GNU:
		return fmt.Sprintf("%s %s", vw.Name, vw.Version)
	case Conventional:
		// Conventional is the default case
		fallthrough
	default:
		// Use the Conventional format, if not recognized
		return fmt.Sprintf("%s version: %s", vw.Name, vw.Version)
	}
}

func (vw VersionWriter) Print(w io.Writer) (n int, err error) {
	return fmt.Fprintf(w, vw.String()+"\n")
}
