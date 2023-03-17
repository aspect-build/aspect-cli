package gazelle

import (
	"fmt"
	"path"

	"github.com/evanw/esbuild/pkg/api"
)

type Parser struct {
}

func NewParser() *Parser {
	p := &Parser{}
	return p
}

// filenameToLoader takes in a filename, e.g. myFile.ts,
// and returns the appropriate esbuild loader for that file.
func filenameToLoader(filename string) api.Loader {
	ext := path.Ext(filename)
	switch ext {
	case ".tsx":
		return api.LoaderTSX
	case ".js":
		return api.LoaderJSX
	case ".jsx":
		return api.LoaderJSX
	default:
		return api.LoaderTS
	}
}

// ParseImports returns all the imports from a file after parsing it.
func (p *Parser) ParseImports(filePath, source string) ([]string, []error) {
	BazelLog.Tracef("ParseImports %s", filePath)

	imports := []string{}

	// Construct an esbuild plugin that pulls out all the imports.
	plugin := api.Plugin{
		Name: "GetImports",
		Setup: func(pluginBuild api.PluginBuild) {
			// callback is a handler for esbuild resolutions. This is how
			// we'll get access to every import in the file.
			callback := func(args api.OnResolveArgs) (api.OnResolveResult, error) {
				// Add the imported string to our list of imports.
				imports = append(imports, args.Path)
				return api.OnResolveResult{
					// Mark the import as external so esbuild doesn't complain
					// about not being able to find the import.
					External: true,
				}, nil
			}

			// pluginBuild.OnResolve sets the plugin's onResolve callback to our custom callback.
			// Make sure to set Filter: ".*" so that our plugin runs on all imports.
			pluginBuild.OnResolve(api.OnResolveOptions{Filter: ".*", Namespace: ""}, callback)
		},
	}
	options := api.BuildOptions{
		Stdin: &api.StdinOptions{
			Contents:   source,
			Sourcefile: filePath,
			// The Loader determines how esbuild will parse the file.
			// We want to parse .ts files as typescript, .tsx files as .tsx, etc.
			Loader: filenameToLoader(filePath),
		},
		Plugins: []api.Plugin{
			plugin,
		},
		// Must set bundle to true so that esbuild actually does resolutions.
		Bundle: true,
		// Must include unused imports that would normally be tree-shacken
		IgnoreAnnotations: true,
		// No need to process anything for sourcemaps
		Sourcemap: api.SourceMapNone,
	}
	result := api.Build(options)

	// Record and return errors alongside the imports
	if 0 < len(result.Errors) {
		errors := []error{}

		for _, e := range result.Errors {
			errors = append(errors, fmt.Errorf(fmt.Sprintf("%v: %s", e.Location.Line, e.Text)))
		}

		return imports, errors
	}

	return imports, nil
}
