package gazelle

import (
	"testing"
)

func TestPnpmLockParseDependencies(t *testing.T) {
	t.Run("lockfile version", func(t *testing.T) {
		v, e := parsePnpmLockVersion([]byte("lockfileVersion: 5.4"))
		if e != nil {
			t.Error(e)
		} else if v != "5.4" {
			t.Error("Failed to parse lockfile version 5.4")
		}

		v, e = parsePnpmLockVersion([]byte("lockfileVersion: '6.0'"))
		if e != nil {
			t.Error(e)
		} else if v != "6.0" {
			t.Error("Failed to parse lockfile version 6.0")
		}
	})

	t.Run("empty lock file", func(t *testing.T) {
		emptyLock, err := parsePnpmLockDependencies([]byte(""))
		if err != nil {
			t.Error("Parse failure: ", err)
		}
		if emptyLock == nil {
			t.Error("Empty lock file not parsed")
		}
	})

	t.Run("unsupported version", func(t *testing.T) {
		_, err := parsePnpmLockDependencies([]byte("lockfileVersion: 4.0"))
		if err == nil {
			t.Error("Expected error for unsupported version")
		}

		_, err2 := parsePnpmLockDependencies([]byte("lockfileVersion: '4.0'"))
		if err2 == nil {
			t.Error("Expected error for unsupported version")
		}
	})

	t.Run("basic deps (lockfile v5)", func(t *testing.T) {
		basic, err := parsePnpmLockDependencies([]byte(`
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

		if err != nil {
			t.Error("Parse failure: ", err)
		}

		if len(basic) != 1 || basic["."] == nil {
			t.Error("Simple deps parse error. Expected only '.' workspace, found ", len(basic))
		}

		if len(basic["."]) != 3 {
			t.Error("Simple deps parse error. Expected 3 deps in 1 workspace entry, found ", len(basic["."]))
		}

		if basic["."]["jquery"] != "3.6.1" {
			t.Errorf("Simple deps parse error. Expected 2.0.2 version for @aspect-test/c, found %q", basic["."]["@aspect-test/c"])
		}
	})

	t.Run("basic deps (lockfile v6)", func(t *testing.T) {
		basic, err := parsePnpmLockDependencies([]byte(`
lockfileVersion: '6.0'

dependencies:
  '@aspect-test/a':
    specifier: 5.0.2
    version: 5.0.2
  jquery:
    specifier: 3.6.1
    version: 3.6.1

devDependencies:
  '@aspect-test/c':
    specifier: 2.0.2
    version: 2.0.2
`))

		if err != nil {
			t.Error("Parse failure: ", err)
		}

		if len(basic) != 1 || basic["."] == nil {
			t.Error("Simple deps parse error. Expected only '.' workspace, found ", len(basic))
		}

		if len(basic["."]) != 3 {
			t.Error("Simple deps parse error. Expected 3 deps in 1 workspace entry, found ", len(basic["."]))
		}

		if basic["."]["jquery"] != "3.6.1" {
			t.Errorf("Simple deps parse error. Expected 2.0.2 version for @aspect-test/c, found %q", basic["."]["@aspect-test/c"])
		}
	})

	t.Run("basic deps in single project workspace (lockfile v6)", func(t *testing.T) {
		basic, err := parsePnpmLockDependencies([]byte(`
lockfileVersion: '6.0'

importers:
  .:
    dependencies:
      '@aspect-test/a':
        specifier: 5.0.2
        version: 5.0.2
      jquery:
        specifier: 3.6.1
        version: 3.6.1
    devDependencies:
      '@aspect-test/c':
        specifier: ^2.0.2
        version: 2.0.2
`))

		if err != nil {
			t.Error("Parse failure: ", err)
		}

		if len(basic) != 1 || basic["."] == nil {
			t.Error("Simple deps parse error. Expected only '.' workspace, found ", len(basic))
		}

		if len(basic["."]) != 3 {
			t.Error("Simple deps parse error. Expected 3 deps in 1 workspace entry, found ", len(basic["."]))
		}

		if basic["."]["jquery"] != "3.6.1" {
			t.Errorf("Simple deps parse error. Expected 2.0.2 version for @aspect-test/c, found %q", basic["."]["@aspect-test/c"])
		}
	})

	t.Run("basic deps in single project workspace (lockfile v5)", func(t *testing.T) {
		basic, err := parsePnpmLockDependencies([]byte(`
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

		if err != nil {
			t.Error("Parse failure: ", err)
		}

		if len(basic) != 1 || basic["."] == nil {
			t.Error("Simple deps parse error. Expected only '.' workspace, found ", len(basic))
		}

		if len(basic["."]) != 3 {
			t.Error("Simple deps parse error. Expected 3 deps in 1 workspace entry, found ", len(basic["."]))
		}

		if basic["."]["jquery"] != "3.6.1" {
			t.Errorf("Simple deps parse error. Expected 2.0.2 version for @aspect-test/c, found %q", basic["."]["@aspect-test/c"])
		}
	})

	t.Run("no deps property", func(t *testing.T) {
		empty, err := parsePnpmLockDependencies([]byte(`
lockfileVersion: 5.4
`))

		if err != nil {
			t.Error("Parse failure: ", err)
		}

		if len(empty) != 1 || len(empty["."]) != 0 {
			t.Error("No deps parse error: ", empty)
		}
	})

	t.Run("deps to workspace pkgs (lockfile v5)", func(t *testing.T) {
		wksps, err := parsePnpmLockDependencies([]byte(`
lockfileVersion: 5.3
importers:
  a:
    specifiers:
      '@lib/a': workspace:*
      '@lib/b': ./lib/a
      '@lib/c': file:./lib/a
      '@lib/d': link:./lib/a
      '@lib/e': workspace:@lib/a@*
      '@lib/f': workspace:./lib/a
      '@lib/g': npm:@lib/b@*
    dependencies:
      '@lib/a': link:./lib/a
      '@lib/b': link:./lib/a
      '@lib/c': link:./lib/a
      '@lib/d': link:./lib/a
      '@lib/e': link:./lib/a
      '@lib/f': link:./lib/a
      '@lib/g': link:./lib/a
`,
		))

		if err != nil {
			t.Error("Parse failure: ", err)
		}

		if len(wksps) != 1 || wksps["a"] == nil {
			t.Error("expected 1 importers, found: ", len(wksps))
		}

		for _, lib := range []string{"a", "b", "c", "d", "e", "f", "g"} {
			if wksps["a"]["@lib/"+lib] != "link:./lib/a" {
				t.Error("expected '@lib/a' dep, found: ", wksps["a"]) //["@lib/"+lib])
			}
		}
	})

	t.Run("deps to workspace pkgs (lockfile v6)", func(t *testing.T) {
		wksps, err := parsePnpmLockDependencies([]byte(`
lockfileVersion: '6.1'
importers:
  a:
    dependencies:
      '@lib/a':
        specifier: workspace:*
        version: link:./lib/a
      '@lib/b':
        specifier: workspace:*
        version: link:./lib/a
      '@lib/c':
        specifier: link:./lib/a
        version: link:./lib/a
      '@lib/d':
        specifier: ./lib/a
        version: link:./lib/a
      '@lib/e':
        specifier: npm:@lib/a@*
        version: link:./lib/a
`,
		))

		if err != nil {
			t.Error("Parse failure: ", err)
		}

		if len(wksps) != 1 || wksps["a"] == nil {
			t.Error("expected 1 importers, found: ", len(wksps))
		}

		for _, lib := range []string{"a", "b", "c", "d", "e"} {
			if wksps["a"]["@lib/"+lib] != "link:./lib/a" {
				t.Error("expected '@lib/a' dep, found: ", wksps["a"]["@lib/"+lib])
			}
		}
	})

	t.Run("workspace deps (lockfile v5)", func(t *testing.T) {
		wksps, err := parsePnpmLockDependencies([]byte(`
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

		if err != nil {
			t.Error("Parse failure: ", err)
		}

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
