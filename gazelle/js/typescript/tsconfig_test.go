package typescript

import (
	"testing"
)

func parseTest(t *testing.T, tsconfigJSON string) *TsCompilerOptionsJSON {
	options, err := parseTsConfigJSON("tsconfig_test", []byte(tsconfigJSON))
	if err != nil {
		t.Fatalf("failed to parse options: %v", err)
	}

	return &options.CompilerOptions
}

func TestTypescriptApi(t *testing.T) {
	t.Run("parse a tsconfig with no config", func(t *testing.T) {
		options := parseTest(t, "{}")

		if options.RootDir != "." {
			t.Errorf("ParseTsConfigOptions: RootDir\nactual:   %s\nexpected:  %s\n", options.RootDir, ".")
		}
	})

	t.Run("parse a tsconfig with no compilerOptions", func(t *testing.T) {
		options := parseTest(t, `{"compilerOptions": {}}`)

		if options.RootDir != "." {
			t.Errorf("ParseTsConfigOptions: RootDir\nactual:   %s\nexpected:  %s\n", options.RootDir, ".")
		}
	})

	t.Run("parse a tsconfig with rootDir", func(t *testing.T) {
		options := parseTest(t, `{"compilerOptions": {"rootDir": "src"}}`)

		if options.RootDir != "src" {
			t.Errorf("ParseTsConfigOptions: RootDir\nactual:   %s\nexpected:  %s\n", options.RootDir, "src")
		}
	})

	t.Run("parse a tsconfig with rootDir relative", func(t *testing.T) {
		options := parseTest(t, `{"compilerOptions": {"rootDir": "./src"}}`)

		if options.RootDir != "src" {
			t.Errorf("ParseTsConfigOptions: RootDir\nactual:   %s\nexpected:  %s\n", options.RootDir, "src")
		}
	})

	t.Run("parse a tsconfig with rootDir relative extra dots", func(t *testing.T) {
		options := parseTest(t, `{"compilerOptions": {"rootDir": "./src/./foo/../"}}`)

		if options.RootDir != "src" {
			t.Errorf("ParseTsConfigOptions: RootDir\nactual:   %s\nexpected:  %s\n", options.RootDir, "src")
		}
	})
}
