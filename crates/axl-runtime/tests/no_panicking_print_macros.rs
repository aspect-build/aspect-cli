//! Convention guard: the panicking print macros must not return.
//!
//! Mirrors the same test in `aspect-cli`; each crate scans its own sources.

/// The panicking print macros must not come back.
///
/// `println!` and friends panic when the write fails, and a reader that leaves
/// early makes that routine — see CLAUDE.md, "Writing to stdout/stderr". The
/// tolerant `outln!` / `errln!` / `out!` replace them. This scans the crate's
/// own sources so a reintroduction fails here rather than as a stranded task
/// months later.
///
/// Sources are read from a compile-time embed rather than the filesystem so the
/// test passes under Bazel's sandbox, matching `loads_use_canonical_public_private_form`.
#[test]
fn no_panicking_print_macros() {
    use include_dir::{Dir, include_dir};
    static SRC: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src");

    let mut offenders = Vec::new();
    let mut stack = vec![&SRC];
    while let Some(dir) = stack.pop() {
        for entry in dir.entries() {
            match entry {
                include_dir::DirEntry::Dir(d) => stack.push(d),
                include_dir::DirEntry::File(f) => {
                    let path = f.path().to_string_lossy().to_string();
                    if !path.ends_with(".rs") {
                        continue;
                    }
                    let Some(text) = f.contents_utf8() else {
                        continue;
                    };
                    // `out.rs` defines the replacements in terms of the real
                    // writers; test modules may panic freely.
                    if path.ends_with("out.rs") {
                        continue;
                    }
                    let mut in_tests = false;
                    for (n, line) in text.lines().enumerate() {
                        if line.trim_start().starts_with("#[cfg(test)]") {
                            in_tests = true;
                        }
                        if in_tests {
                            continue;
                        }
                        let t = line.trim_start();
                        if t.starts_with("//") || t.starts_with("///") || t.starts_with("//!") {
                            continue;
                        }
                        for m in ["println!(", "eprintln!(", "print!(", "eprint!("] {
                            // `outln!(` ends in `print!(`-free text; match the
                            // macro at a token boundary.
                            let is_call = line.contains(m)
                                && !line.contains(&format!("out{m}"))
                                && !line.contains(&format!("err{m}"));
                            if is_call {
                                offenders.push(format!("{path}:{}: {}", n + 1, t));
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "use outln!/errln!/out! from axl_runtime::out instead (see CLAUDE.md):\n  {}",
        offenders.join("\n  ")
    );
}
