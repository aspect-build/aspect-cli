package gazelle

import (
	"log"
	"os"

	"gopkg.in/yaml.v3"
)

func ParsePnpmLockFileDependencies(lockfilePath string) map[string]map[string]string {
	yamlFile, readErr := os.ReadFile(lockfilePath)
	if readErr != nil {
		log.Fatalf("failed to read pnpm '%s': %s", lockfilePath, readErr.Error())
		os.Exit(1)
	}

	return parsePnpmLockDependencies(yamlFile)
}

func parsePnpmLockDependencies(yamlFileContent []byte) map[string]map[string]string {
	lockfile := PnpmLockfile{}

	unmarchalErr := yaml.Unmarshal(yamlFileContent, &lockfile)
	if unmarchalErr != nil {
		log.Fatalln("Failed parse pnpm lockfile: ", unmarchalErr)
		os.Exit(1)
	}

	result := make(map[string]map[string]string)

	// Pnpm workspace lockfiles contain a list of projects under Importers, including
	// the root project as the "." importer.
	if lockfile.Importers != nil {
		for pkg, pkgDeps := range lockfile.Importers {

			if result[pkg] != nil {
				log.Fatalln("Invalid pnpm lockfile, duplicate importer: ", pkg)
			}

			result[pkg] = mergeDependencies(pkgDeps.Dependencies, pkgDeps.DevDependencies, pkgDeps.PeerDependencies, pkgDeps.OptionalDependencies)
		}
	} else {
		// Non-workspace lockfiles have one set of dependencies at the root
		result["."] = mergeDependencies(lockfile.Dependencies, lockfile.DevDependencies, lockfile.PeerDependencies, lockfile.OptionalDependencies)
	}

	return result
}

/*
	  Example pnpm-lock.yaml with workspaces

	  ```
		lockfileVersion: 5.4
		specifiers:
			'@aspect-test/c': ^2.0.2
	  		jquery: 3.6.1
		dependencies:
			'@aspect-test/c': 2.0.2
		devDependencies:
			jquery: 3.6.1
		packages:
			/@aspect-test/c/2.0.2:
				...
			...
		...
	  ```

	  or with pnpm-workspaces.yaml:

	  ```
		lockfileVersion: 5.4
		importers:
			.:
				specifiers:
					'@aspect-test/a': ^2.0.2
				dependencies:
					'@aspect-test/a': ^2.0.2
			gazelle/ts/tests/simple_json_import:
				specifiers: {}
			infrastructure/cdn:
				specifiers:
					'@aspect-test/c': ^2.0.2
				dependencies:
					'@aspect-test/c': ^2.0.2
		packages:
			/@aspect-test/c/2.0.2:
				...
			...
		...
	  ```
*/
type PnpmLockfile struct {
	Dependencies         map[string]string
	DevDependencies      map[string]string `yaml:"devDependencies"`
	PeerDependencies     map[string]string `yaml:"peerDependencies"`
	OptionalDependencies map[string]string `yaml:"optionalDependencies"`

	Importers map[string]struct {
		Dependencies         map[string]string
		DevDependencies      map[string]string `yaml:"devDependencies"`
		PeerDependencies     map[string]string `yaml:"peerDependencies"`
		OptionalDependencies map[string]string `yaml:"optionalDependencies"`
	}
}

func mergeDependencies(d ...map[string]string) map[string]string {
	result := make(map[string]string)

	for _, m := range d {
		for k, v := range m {
			result[k] = v
		}
	}

	return result
}
