package gazelle

import (
	"os"
	"path"

	BazelLog "aspect.build/cli/pkg/logger"
	"github.com/msolo/jsonr"
)

type npmPackageJSON struct {
	// main: https://nodejs.org/docs/latest-v22.x/api/packages.html#main
	Main string `json:"main"`

	// exports: https://nodejs.org/docs/latest-v22.x/api/packages.html#exports
	Exports interface{} `json:"exports"`
}

// Extract the various import types from the package.json file such as
// 'main' and 'exports' fields.
func ParsePackageJsonImportsFile(rootDir, packageJsonPath string) ([]string, error) {
	content, err := os.ReadFile(path.Join(rootDir, packageJsonPath))
	if err != nil {
		return nil, err
	}

	return parsePackageJsonImports(content)
}

func parsePackageJsonImports(packageJsonContent []byte) ([]string, error) {
	var c npmPackageJSON
	if err := jsonr.Unmarshal(packageJsonContent, &c); err != nil {
		return nil, err
	}

	imports := make([]string, 0)

	if c.Main != "" {
		imports = append(imports, path.Clean(c.Main))
	}

	if c.Exports != nil {
		switch exports := c.Exports.(type) {
		case string:
			// Single export
			imports = append(imports, path.Clean(exports))
		case map[string]interface{}:
			// Subpath exports
			for exportKey, export := range exports {
				switch e := export.(type) {
				case string:
					// Regular subpath export
					imports = append(imports, path.Clean(e))
				case map[string]interface{}:
					// Conditional subpath export
					for subEKey, subE := range e {
						switch subE := subE.(type) {
						case string:
							imports = append(imports, path.Clean(subE))
						default:
							BazelLog.Warnf("unknown exports.%s.%s type: %T", exportKey, subEKey, subE)
						}
					}
				default:
					BazelLog.Warnf("unknown exports.%s type: %T", exportKey, export)
				}
			}
		default:
			BazelLog.Warnf("unknown exports type: %T", exports)
		}
	}

	return imports, nil
}
