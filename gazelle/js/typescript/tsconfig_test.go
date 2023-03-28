package typescript

import (
	"reflect"
	"testing"
)

func parseTest(t *testing.T, tsconfigJSON string) *TsConfig {
	cm := &TsConfigMap{
		configs: make(map[string]*TsConfig),
	}

	options, err := parseTsConfigJSON(cm, ".", "tsconfig_test", []byte(tsconfigJSON))
	if err != nil {
		t.Fatalf("failed to parse options: %v\n\n%s", err, tsconfigJSON)
	}

	return options
}

func assertExpand(t *testing.T, options *TsConfig, p string, expected ...string) {
	actual := options.ExpandPaths(".", p)

	// TODO: why does reflect.DeepEqual not handle this case?
	if len(actual) == 0 && len(expected) == 0 {
		return
	}

	if !reflect.DeepEqual(actual, expected) {
		t.Errorf("TsConfig ExpandPath(%q):\n\texpected: %v\n\tactual:   %v", p, expected, actual)
	}
}

func TestTypescriptApi(t *testing.T) {
	t.Run("parse a tsconfig with empty config", func(t *testing.T) {
		options := parseTest(t, "{}")

		if options.RootDir != "." {
			t.Errorf("ParseTsConfigOptions: RootDir\nactual:   %s\nexpected:  %s\n", options.RootDir, ".")
		}
	})

	t.Run("parse a tsconfig with empty compilerOptions", func(t *testing.T) {
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

	t.Run("parse tsconfig files with relaxed json", func(t *testing.T) {
		parseTest(t, `{}`)
		parseTest(t, `{"compilerOptions": {}}`)
		parseTest(t, `
			{
				"compilerOptions": {
					"rootDir": "src",
					"baseUrl": ".",
				},
			}
		`)
		parseTest(t, `
			{
				"compilerOptions": {
					// line comment
					"paths": {
						"x": ["./y.ts"], // trailing with comments
					},
					"baseUrl": ".",
				},
			}
		`)
	})

	t.Run("tsconfig paths expansion basic", func(t *testing.T) {
		// Initial request: https://github.com/aspect-build/aspect-cli/issues/396
		config := parseTest(t, `{
			"compilerOptions": {
			  "declaration": true,
			  "baseUrl": ".",
			  "paths": {
				"@org/*": [
				  "b/src/*"
				]
			  }
			}
		  }`)

		assertExpand(t, config, "@org/lib", "b/src/lib")
	})

	t.Run("tsconfig paths expansion", func(t *testing.T) {
		config := parseTest(t, `{
				"compilerOptions": {
					"baseUrl": ".",
					"paths": {
						"test0": ["./test0-success.ts"],
						"test1/*": ["./test1-success.ts"],
						"test2/*": ["./test2-success/*"],
						"t*t3/foo": ["./test3-succ*s.ts"],
						"test4/*": ["./test4-first/*", "./test4-second/*"],
						"test5/*": ["./test5-first/*", "./test5-second/*"]
					}
				}
			}`)

		assertExpand(t, config, "test0", "test0-success.ts")
		assertExpand(t, config, "test1/bar", "test1-success.ts")
		assertExpand(t, config, "test1/foo", "test1-success.ts")
		assertExpand(t, config, "test2/foo", "test2-success/foo")
		assertExpand(t, config, "test3/x")

		assertExpand(t, config, "tXt3/foo", "test3-succXs.ts")
		assertExpand(t, config, "t123t3/foo", "test3-succ123s.ts")
		assertExpand(t, config, "t-t3/foo", "test3-succ-s.ts")

		assertExpand(t, config, "test4/x", "test4-first/x", "test4-second/x")
	})

	t.Run("tsconfig paths expansion star-length tie-breaker", func(t *testing.T) {
		config := parseTest(t, `{
				"compilerOptions": {
					"baseUrl": ".",
					"paths": {
						"lib/*": ["fallback/*"],
						"lib/a": ["a-direct"],
						"l*": ["l-star/*"],
						"lib*": ["lib-star/*"],
						"li*": ["li-star/*"],
						"lib*-suff": ["lib-star-suff/*"]
					}
				}
			}`)

		assertExpand(t, config, "lib/a", "a-direct", "fallback/a", "lib-star/a", "li-star/b/a", "l-star/ib/a")
	})
}
