package typescript

import (
	"encoding/json"
	"os"
	"path"
)

type TsCompilerOptionsJSON struct {
	RootDir string `json:"rootDir"`
}

type TsConfigJSON struct {
	CompilerOptions TsCompilerOptionsJSON `json:"compilerOptions"`
}

// parseTsConfigJSONFile loads a tsconfig.json file and return the compilerOptions config
func parseTsConfigJSONFile(tsconfigPath string) (*TsConfigJSON, error) {
	content, readErr := os.ReadFile(tsconfigPath)
	if readErr != nil {
		return nil, readErr
	}

	return parseTsConfigJSON(tsconfigPath, content)
}

func parseTsConfigJSON(tsconfigPath string, tsconfigJSON []byte) (*TsConfigJSON, error) {
	// TODO: support relaxed json syntax such as trailing commas
	// See https://github.com/tailscale/hujson

	var c TsConfigJSON
	if err := json.Unmarshal(tsconfigJSON, &c); err != nil {
		return nil, err
	}

	// Normalize paths
	c.CompilerOptions.RootDir = path.Clean(c.CompilerOptions.RootDir)

	return &c, nil
}
