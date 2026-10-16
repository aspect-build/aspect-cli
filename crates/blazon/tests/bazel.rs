//! The Starlark annotation model on Bazel 9.0.1's own classes. Skipped when
//! the release is not in the buildx cache.

use blazon::classfile::Classes;
use blazon::jar::Jar;
use std::path::PathBuf;

fn classes() -> Option<Classes> {
    let jar = dirs_cache()?.join("buildx/bazel/9.0.1/A-server.jar");
    jar.exists()
        .then(|| Classes::new(Jar::open(&jar).expect("jar")))
}

fn dirs_cache() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Caches"))
}

fn dump(java: &str) -> Option<String> {
    let mut c = classes()?;
    Some(blazon::dump::class(&mut c, java).expect("dump"))
}

#[test]
fn label() {
    let Some(d) = dump("com.google.devtools.build.lib.cmdline.Label") else {
        return;
    };
    assert!(d.contains("starlark builtin \"Label\""), "{d}");
    for name in ["name", "package", "workspace_name", "repo_name"] {
        assert!(
            d.lines()
                .any(|l| l.contains(&format!("starlark method {name}()"))
                    && l.contains("structField")),
            "{name}: {d}"
        );
    }
    let relative = d
        .lines()
        .find(|l| l.contains("starlark method relative("))
        .expect("relative");
    assert!(relative.contains("relName [pos]"), "{relative}");
    assert!(relative.contains("useStarlarkThread"), "{relative}");
    assert!(relative.contains("getRelative"), "{relative}");
}

#[test]
fn starlark_list_is_a_sequence() {
    let Some(d) = dump("net.starlark.java.eval.StarlarkList") else {
        return;
    };
    assert!(d.contains("starlark builtin \"list\""), "{d}");
    let interfaces = d
        .lines()
        .find(|l| l.contains("starlark interfaces"))
        .expect("interfaces");
    for i in [
        "Sequence",
        "StarlarkIndexable",
        "StarlarkIterable",
        "StarlarkMembershipTestable",
    ] {
        assert!(interfaces.contains(i), "{interfaces}");
    }
    assert!(d.contains("starlark method append(item [pos])"), "{d}");
    let index = d
        .lines()
        .find(|l| l.contains("starlark method index("))
        .expect("index");
    assert!(index.contains("start: StarlarkInt = unbound"), "{index}");
}

#[test]
fn dict_and_struct() {
    let Some(d) = dump("net.starlark.java.eval.Dict") else {
        return;
    };
    assert!(d.contains("starlark builtin \"dict\""), "{d}");
    let get = d
        .lines()
        .find(|l| l.contains("starlark method get("))
        .expect("get");
    assert!(
        get.contains("default = None") && get.contains("useStarlarkThread"),
        "{get}"
    );
    let update = d
        .lines()
        .find(|l| l.contains("starlark method update("))
        .expect("update");
    assert!(update.contains("**kwargs"), "{update}");

    let Some(s) = dump("com.google.devtools.build.lib.packages.StructImpl") else {
        return;
    };
    assert!(s.contains("starlark builtin \"struct\""), "{s}");
    assert!(s.contains("Structure"), "{s}");
}
