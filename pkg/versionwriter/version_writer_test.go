package versionwriter_test

import (
	"testing"

	"aspect.build/cli/buildinfo"
	"aspect.build/cli/pkg/versionwriter"
	"github.com/stretchr/testify/assert"
)

const (
	name    = "Chicken"
	version = "1.2.3"
)

func TestNew(t *testing.T) {
	format := versionwriter.Conventional
	actual := versionwriter.New(name, version, format)
	expected := versionwriter.VersionWriter{
		Name:    name,
		Version: version,
		Format:  format,
	}
	assert.Equal(t, expected, actual)
}

func TestNewFromBuildInfo(t *testing.T) {
	name := "Chicken"
	format := versionwriter.Conventional
	actual := versionwriter.NewFromBuildInfo(name, format)
	assert.Equal(t, name, actual.Name)
	assert.Equal(t, format, actual.Format)
	assert.Contains(t, actual.Version, buildinfo.Release)
}

func TestString(t *testing.T) {
	t.Error("IMPLEMENT ME!")
}

func TestPrint(t *testing.T) {
	t.Error("IMPLEMENT ME!")
}
