package gazelle

import (
	"os"
	"path"

	"github.com/msolo/jsonr"
)

type NpmPackageJSON struct {
	Main    string            `json:"main"`
	Exports map[string]string `json:"exports"`
}

func ParsePackageJsonImportsFile(rootDir, packageJsonPath string) ([]string, error) {
	content, err := os.ReadFile(path.Join(rootDir, packageJsonPath))
	if err != nil {
		return nil, err
	}

	return parsePackageJsonImports(content)
}

func parsePackageJsonImports(packageJsonContent []byte) ([]string, error) {
	var c NpmPackageJSON
	if err := jsonr.Unmarshal(packageJsonContent, &c); err != nil {
		return nil, err
	}

	imports := make([]string, 0)

	if c.Main != "" {
		imports = append(imports, path.Clean(c.Main))
	}

	if c.Exports != nil {
		for _, v := range c.Exports {
			imports = append(imports, path.Clean(v))
		}
	}

	return imports, nil
}
