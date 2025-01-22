package runfiles_test

import (
	"github.com/aspect-build/aspect-cli/pkg/bazel/runfiles"
	"github.com/stretchr/testify/require"
	"testing"
)

func TestUnescape(t *testing.T) {
	tests := []struct {
		input    string
		expected string
	}{
		{"hello\\sworld", "hello world"},
		{"new\\nline", "new\nline"},
		{"back\\bslash", "back\\slash"},
		{"double\\\\slash", "double\\slash"},
		{"raw\\xsequence", "raw\\xsequence"},
		{"no\\", "no\\"}, // Handles trailing backslash
		{"\\s\\n\\b", " \n\\"},
		{"", ""},
		{"normal text", "normal text"},
		{"\\s\\s\\s", "   "},
	}

	for _, tt := range tests {
		t.Run(tt.input, func(t *testing.T) {
			got := runfiles.Unescape(tt.input)
			require.Equal(t, tt.expected, got)
		})
	}
}
