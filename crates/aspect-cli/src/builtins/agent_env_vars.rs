//! `axl_runtime::agent::ENV_VARS` names exactly the environment variables
//! `lib/agent.axl` reads to detect a coding agent.
//!
//! The runtime picks the `agent` Console layout from those variables without
//! running AXL, so it keeps its own list; this reads the AXL source so a
//! harness added on one side and not the other fails here instead of drifting.
//! The source is embedded at compile time, as the CLI embeds it to ship it.

use std::collections::BTreeSet;

use include_dir::{Dir, include_dir};

static ASPECT: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/builtins/aspect");

/// The first non-empty `"…"` literal after `prefix` on each line where
/// `prefix` occurs, at the line's start (after indentation) when `leading`.
fn quoted_after(source: &str, prefix: &str, leading: bool) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| match leading {
            true => line.trim_start().strip_prefix(prefix),
            false => line.split_once(prefix).map(|(_, rest)| rest),
        })
        .filter_map(|rest| rest.split('"').nth(1))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_runtime_reads_the_same_agent_variables_as_agent_axl() {
    let source = ASPECT
        .get_file("private/lib/agent.axl")
        .and_then(|f| f.contents_utf8())
        .expect("private/lib/agent.axl");
    let mut axl: BTreeSet<String> = BTreeSet::new();
    for prefix in ["marker = ", "id_var = ", "_GENERIC_KIND_VAR = "] {
        axl.extend(quoted_after(source, prefix, true));
    }
    axl.extend(
        quoted_after(source, "std.env.var(", false)
            .into_iter()
            .filter(|name| name.starts_with("ASPECT_AGENT_")),
    );
    let rust: BTreeSet<String> = axl_runtime::agent::ENV_VARS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    assert_eq!(rust, axl);
}
