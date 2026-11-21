//! Bazel's `StableSort` of an expanded execution log, which makes logs from
//! nondeterministic runs of one build diff cleanly: a spawn that produces
//! another's input comes before it, and otherwise spawns go in order of their
//! first listed output.
//!
//! Spawns with no listed output are ordered by their stable fields, as Bazel
//! orders them by those fields' text format; the text differs, so such spawns
//! may come in a different order from Bazel's among themselves. Bazel assumes
//! no cycles and would drop the spawns in one; they come last here instead.

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap, HashMap};

use axl_proto::tools::protos::SpawnExec;

/// `spawns` in Bazel's stable order.
pub fn stable_sort(spawns: Vec<SpawnExec>) -> Vec<SpawnExec> {
    let keys: Vec<String> = spawns.iter().map(key).collect();
    let mut producers: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, spawn) in spawns.iter().enumerate() {
        for output in &spawn.actual_outputs {
            producers.entry(output.path.as_str()).or_default().push(i);
        }
    }

    // `blocked_by[b]`: the spawns producing an input of `b`.
    let mut blocked_by: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); spawns.len()];
    let mut blocking: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); spawns.len()];
    let mut ready = BinaryHeap::new();
    for (i, spawn) in spawns.iter().enumerate() {
        for input in &spawn.inputs {
            for &producer in producers.get(input.path.as_str()).into_iter().flatten() {
                blocked_by[i].insert(producer);
                blocking[producer].insert(i);
            }
        }
        if blocked_by[i].is_empty() {
            ready.push(Reverse((&keys[i], i)));
        }
    }

    let mut order = Vec::with_capacity(spawns.len());
    let mut emitted = vec![false; spawns.len()];
    while let Some(Reverse((_, current))) = ready.pop() {
        order.push(current);
        emitted[current] = true;
        for &blocked in &blocking[current] {
            blocked_by[blocked].remove(&current);
            if blocked_by[blocked].is_empty() {
                ready.push(Reverse((&keys[blocked], blocked)));
            }
        }
    }
    order.extend((0..spawns.len()).filter(|&i| !emitted[i]));

    let mut spawns: Vec<Option<SpawnExec>> = spawns.into_iter().map(Some).collect();
    order.into_iter().filter_map(|i| spawns[i].take()).collect()
}

/// A spawn's sort key: its first listed output's path, else its stable
/// fields (never its outputs' digests, so one action graph sorts one way
/// whatever the files hold).
fn key(spawn: &SpawnExec) -> String {
    match spawn.listed_outputs.first() {
        Some(output) => format!("1_{output}"),
        None => format!(
            "2_{:?}",
            SpawnExec {
                command_args: spawn.command_args.clone(),
                environment_variables: spawn.environment_variables.clone(),
                platform: spawn.platform.clone(),
                inputs: spawn.inputs.clone(),
                mnemonic: spawn.mnemonic.clone(),
                ..SpawnExec::default()
            }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_proto::tools::protos::File;

    fn spawn(name: &str, inputs: &[&str], outputs: &[&str]) -> SpawnExec {
        let file = |path: &&str| File {
            path: path.to_string(),
            ..File::default()
        };
        SpawnExec {
            mnemonic: name.to_string(),
            inputs: inputs.iter().map(file).collect(),
            listed_outputs: outputs.iter().map(|s| s.to_string()).collect(),
            actual_outputs: outputs.iter().map(file).collect(),
            ..SpawnExec::default()
        }
    }

    fn names(spawns: &[SpawnExec]) -> Vec<&str> {
        spawns.iter().map(|s| s.mnemonic.as_str()).collect()
    }

    #[test]
    fn producers_come_first_then_by_first_output() {
        let sorted = stable_sort(vec![
            spawn("c", &["out/b"], &["out/c"]),
            spawn("b", &["out/a"], &["out/b"]),
            spawn("z", &[], &["out/0"]),
            spawn("a", &[], &["out/a"]),
        ]);
        assert_eq!(names(&sorted), ["z", "a", "b", "c"]);
    }

    #[test]
    fn spawns_without_outputs_come_after_those_with() {
        let sorted = stable_sort(vec![
            spawn("none", &[], &[]),
            spawn("some", &[], &["out/x"]),
        ]);
        assert_eq!(names(&sorted), ["some", "none"]);
    }

    #[test]
    fn a_cycle_keeps_its_spawns_at_the_end() {
        let sorted = stable_sort(vec![
            spawn("a", &["out/b"], &["out/a"]),
            spawn("b", &["out/a"], &["out/b"]),
            spawn("free", &[], &["out/z"]),
        ]);
        assert_eq!(names(&sorted), ["free", "a", "b"]);
    }
}
