package buildinfo_test

import (
	"testing"

	"aspect.build/cli/buildinfo"
	"github.com/stretchr/testify/assert"
)

const (
	name    = "Chicken"
	version = "1.2.3"
)

func TestUtilityVersion(t *testing.T) {
	t.Run("with conventional format", func(t *testing.T) {
		actual := buildinfo.UtilityVersion(name, version, buildinfo.ConventionalFormat)
		assert.Equal(t, "Chicken version: 1.2.3", actual)
	})
	t.Run("with GNU format", func(t *testing.T) {
		actual := buildinfo.UtilityVersion(name, version, buildinfo.GNUFormat)
		assert.Equal(t, "Chicken 1.2.3", actual)
	})
}
