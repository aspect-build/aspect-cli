use include_dir::{Dir, include_dir};

pub(crate) static STD_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/builtins/std");
pub(crate) static BAZEL_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/builtins/bazel");

/// Proto-derived `@bazel//proto/<...>/<pkg>.axl` shims, synthesized on
/// load. The content is always the same one-line re-export pattern
/// (`<pkg> = _proto.<pkg>`) so the file doesn't need to exist on disk —
/// the lookup is keyed off the path's basename, matching the
/// strip-prefix convention enforced by `engine/mod.rs`.
///
/// To expose a new proto package: register it under `_proto.<pkg>` in
/// `engine/mod.rs` and add the basename + one-liner here. Adding a new
/// entry is the only edit required; no .axl file or `include_dir!`
/// re-scan needed.
const PROTO_SHIMS: &[(&str, &str)] = &[
    ("v2", "v2 = _proto.v2\n"),
    ("bytestream", "bytestream = _proto.bytestream\n"),
    ("longrunning", "longrunning = _proto.longrunning\n"),
    ("rpc", "rpc = _proto.rpc\n"),
    ("semver", "semver = _proto.semver\n"),
    ("remote_logging", "remote_logging = _proto.remote_logging\n"),
];

pub fn get(module: &str, filename: &str) -> Option<&'static str> {
    if module == "bazel" && filename.starts_with("proto/") && filename.ends_with(".axl") {
        // Pull the basename — last path component before `.axl` — and
        // look it up against the known proto packages. The intermediate
        // path components (e.g. `remote/execution/` in
        // `proto/remote/execution/v2.axl`) carry no information beyond
        // the basename; they exist for human navigability of the load
        // path.
        let basename = std::path::Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())?;
        if let Some(&(_, content)) = PROTO_SHIMS.iter().find(|(name, _)| *name == basename) {
            return Some(content);
        }
    }
    let dir = match module {
        "std" => &STD_DIR,
        "bazel" => &BAZEL_DIR,
        _ => return None,
    };
    dir.get_file(filename)?.contents_utf8()
}
