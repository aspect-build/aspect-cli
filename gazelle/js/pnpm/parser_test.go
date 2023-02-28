package gazelle

import (
	"testing"
)

func TestPnpmLockParseDependencies(t *testing.T) {
	t.Run("empty lock file", func(t *testing.T) {
		emptyLock := parsePnpmLockDependencies([]byte(""))
		if emptyLock == nil {
			t.Error("Empty lock file not parsed")
		}
	})

	t.Run("basic deps", func(t *testing.T) {
		basic := parsePnpmLockDependencies([]byte(`
lockfileVersion: 5.4

specifiers:
  '@aspect-test/a': 5.0.2
  '@aspect-test/c': 2.0.2
  jquery: 3.6.1

dependencies:
  '@aspect-test/a': 5.0.2

devDependencies:
  '@aspect-test/c': 2.0.2

peerDependencies:
  jquery: 3.6.1
`))

		if len(basic) != 1 || basic["."] == nil {
			t.Error("Simple deps parse error. Expected only '.' workspace, found ", len(basic))
		}

		if len(basic["."]) != 3 {
			t.Error("Simple deps parse error. Expected 3 deps in 1 workspace entry, found ", len(basic["."]))
		}
	})

	t.Run("basic deps in single project workspace", func(t *testing.T) {
		basic := parsePnpmLockDependencies([]byte(`
lockfileVersion: 5.4

importers:
  .:
    specifiers:
      '@aspect-test/a': 5.0.2
      '@aspect-test/c': 2.0.2
      jquery: 3.6.1

    dependencies:
      '@aspect-test/a': 5.0.2
      '@aspect-test/c': 2.0.2
      jquery: 3.6.1
`))

		if len(basic) != 1 || basic["."] == nil {
			t.Error("Simple deps parse error. Expected only '.' workspace, found ", len(basic))
		}

		if len(basic["."]) != 3 {
			t.Error("Simple deps parse error. Expected 3 deps in 1 workspace entry, found ", len(basic["."]))
		}
	})

	t.Run("no deps property", func(t *testing.T) {
		empty := parsePnpmLockDependencies([]byte(`
lockfileVersion: 5.4
`))

		if len(empty) != 1 || len(empty["."]) != 0 {
			t.Error("No deps parse error: ", empty)
		}
	})

	t.Run("workspace deps", func(t *testing.T) {
		wksps := parsePnpmLockDependencies([]byte(`
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
`))

		if len(wksps) != 3 || wksps["."] == nil || wksps["gazelle/ts/tests/simple_json_import"] == nil || wksps["infrastructure/cdn"] == nil {
			t.Error("expected 3 importers, found: ", len(wksps))
		}

		if len(wksps["."]) != 1 || wksps["."]["@aspect-test/a"] == "" {
			t.Error("expected main importer to have '@aspect-test/a' dep, found: ", wksps["."])
		}

		if len(wksps["gazelle/ts/tests/simple_json_import"]) != 0 {
			t.Error("expected 'gazelle/ts/tests/simple_json_import' importer to have no deps, found ", len(wksps["gazelle/ts/tests/simple_json_import"]))
		}

		if len(wksps["infrastructure/cdn"]) != 1 || wksps["infrastructure/cdn"]["@aspect-test/c"] == "" {
			t.Error("expected 'infrastructure/cdn' importer to have '@aspect-test/c' dep, found: ", wksps["infrastructure/cdn"])
		}
	})
}
