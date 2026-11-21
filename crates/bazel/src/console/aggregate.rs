//! Aggregating the console lines a pattern matches, for a report built from
//! all of them rather than from a sample: [`Spec`] says what to keep, a
//! [`Tally`] keeps it.
//!
//! Matches split into groups by the values of the pattern's `group_by`
//! named captures; each group counts its matches exactly and keeps its
//! [`Aggregation`]s, named, over a capture (or the whole line, `line`).
//! Memory is bounded whatever the console's size: lists stop growing at
//! their `max` and say so, a pattern keeps at most `max_groups` groups
//! (matches of the groups past it are counted, not kept), and the first
//! `samples` matches are kept as they were, in order, for whoever reports
//! them one by one.

use std::collections::{HashMap, HashSet};

use regex::Regex;
use serde_json::{Map, Value, json};

use super::report::cut;

/// The longest copy of a value or line a [`Tally`] keeps.
pub const MAX_VALUE_BYTES: usize = 1024;

/// The most distinct values [`Aggregation::CountDistinct`] tracks, and the
/// most groups past `max_groups` counted.
pub const MAX_DISTINCT: usize = 10_000;

/// The most matches a [`Spec`] keeps as samples.
pub const MAX_SAMPLES: usize = 10_000;

/// The capture name that is the whole matched line.
pub const LINE: &str = "line";

/// What to keep of one capture across a group's matches.
#[derive(Debug, Clone, PartialEq)]
pub enum Aggregation {
    /// The group's matches.
    Count,
    /// The capture's distinct values, first seen first, up to `max`.
    Unique {
        capture: String,
        max: usize,
    },
    /// Every value, repeats included, up to `max`.
    All {
        capture: String,
        max: usize,
    },
    /// How many distinct values, exact up to [`MAX_DISTINCT`].
    CountDistinct {
        capture: String,
    },
    /// Each value and its matches, most first: the first `max` distinct
    /// values seen, each counted exactly.
    CountBy {
        capture: String,
        max: usize,
    },
    First {
        capture: String,
    },
    Last {
        capture: String,
    },
}

impl Aggregation {
    /// The capture it reads, if any.
    pub fn capture(&self) -> Option<&str> {
        match self {
            Aggregation::Count => None,
            Aggregation::Unique { capture, .. }
            | Aggregation::All { capture, .. }
            | Aggregation::CountDistinct { capture }
            | Aggregation::CountBy { capture, .. }
            | Aggregation::First { capture }
            | Aggregation::Last { capture } => Some(capture),
        }
    }
}

/// One pattern's aggregation, as `bazel.output.aggregate(...)` configures it.
#[derive(Debug, Clone)]
pub struct Spec {
    pub id: String,
    pub regex: Regex,
    pub group_by: Vec<String>,
    pub aggregations: Vec<(String, Aggregation)>,
    pub samples: usize,
    pub max_groups: usize,
}

/// Why a [`Spec`] is not valid.
#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("{id}: {source}")]
    Pattern {
        id: String,
        #[source]
        source: regex::Error,
    },
    #[error(
        "{id}: a named group `line` would hide the whole matched line, which `line` names; rename it"
    )]
    LineGroup { id: String },
    #[error("{id}: the pattern has no named group `{capture}` (it has: {have})")]
    NoCapture {
        id: String,
        capture: String,
        have: String,
    },
}

impl Spec {
    /// A spec whose captures are all named groups of `pattern` or [`LINE`].
    pub fn new(
        id: String,
        pattern: &str,
        group_by: Vec<String>,
        aggregations: Vec<(String, Aggregation)>,
        samples: usize,
        max_groups: usize,
    ) -> Result<Spec, SpecError> {
        let regex = Regex::new(pattern).map_err(|source| SpecError::Pattern {
            id: id.clone(),
            source,
        })?;
        let names: Vec<&str> = regex.capture_names().flatten().collect();
        if names.contains(&LINE) {
            return Err(SpecError::LineGroup { id });
        }
        let wanted = group_by
            .iter()
            .map(String::as_str)
            .chain(aggregations.iter().filter_map(|(_, a)| a.capture()));
        for capture in wanted {
            if capture != LINE && !names.contains(&capture) {
                return Err(SpecError::NoCapture {
                    id,
                    capture: capture.to_string(),
                    have: if names.is_empty() {
                        "none".into()
                    } else {
                        names.join(", ")
                    },
                });
            }
        }
        Ok(Spec {
            id,
            regex,
            group_by,
            aggregations,
            samples: samples.min(MAX_SAMPLES),
            max_groups: max_groups.max(1),
        })
    }
}

#[derive(Debug)]
enum State {
    Count,
    List {
        values: Vec<String>,
        seen: Option<HashSet<String>>,
        max: usize,
        truncated: bool,
    },
    Distinct {
        seen: HashSet<String>,
        truncated: bool,
    },
    CountBy {
        counts: Vec<(String, u64)>,
        index: HashMap<String, usize>,
        max: usize,
        truncated: bool,
    },
    One {
        value: Option<String>,
        last: bool,
    },
}

impl State {
    fn new(a: &Aggregation) -> State {
        match a {
            Aggregation::Count => State::Count,
            Aggregation::Unique { max, .. } => State::List {
                values: vec![],
                seen: Some(HashSet::new()),
                max: *max,
                truncated: false,
            },
            Aggregation::All { max, .. } => State::List {
                values: vec![],
                seen: None,
                max: *max,
                truncated: false,
            },
            Aggregation::CountDistinct { .. } => State::Distinct {
                seen: HashSet::new(),
                truncated: false,
            },
            Aggregation::CountBy { max, .. } => State::CountBy {
                counts: vec![],
                index: HashMap::new(),
                max: *max,
                truncated: false,
            },
            Aggregation::First { .. } => State::One {
                value: None,
                last: false,
            },
            Aggregation::Last { .. } => State::One {
                value: None,
                last: true,
            },
        }
    }

    fn add(&mut self, value: &str) {
        match self {
            State::Count => {}
            State::List {
                values,
                seen,
                max,
                truncated,
            } => {
                if let Some(seen) = seen {
                    if seen.contains(value) {
                        return;
                    }
                    if values.len() < *max {
                        seen.insert(value.to_string());
                    }
                }
                if values.len() < *max {
                    values.push(value.to_string());
                } else {
                    *truncated = true;
                }
            }
            State::Distinct { seen, truncated } => {
                if seen.len() < MAX_DISTINCT {
                    seen.insert(value.to_string());
                } else if !seen.contains(value) {
                    *truncated = true;
                }
            }
            State::CountBy {
                counts,
                index,
                max,
                truncated,
            } => match index.get(value) {
                Some(&i) => counts[i].1 += 1,
                None if counts.len() < *max => {
                    index.insert(value.to_string(), counts.len());
                    counts.push((value.to_string(), 1));
                }
                None => *truncated = true,
            },
            State::One { value: v, last } => {
                if *last || v.is_none() {
                    *v = Some(value.to_string());
                }
            }
        }
    }

    fn report(&self, count: u64) -> (Value, bool) {
        match self {
            State::Count => (json!(count), false),
            State::List {
                values, truncated, ..
            } => (json!(values), *truncated),
            State::Distinct { seen, truncated } => (json!(seen.len()), *truncated),
            State::CountBy {
                counts, truncated, ..
            } => {
                let mut sorted = counts.clone();
                sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
                let map: Map<String, Value> =
                    sorted.into_iter().map(|(k, n)| (k, json!(n))).collect();
                (Value::Object(map), *truncated)
            }
            State::One { value, .. } => (json!(value), false),
        }
    }
}

#[derive(Debug)]
struct Group {
    key: Vec<String>,
    /// Its first matching line, and that match's named captures.
    line: String,
    captures: Vec<(String, String)>,
    /// Its last match.
    last: Option<Sample>,
    count: u64,
    states: Vec<State>,
}

#[derive(Debug, Clone)]
struct Sample {
    group: usize,
    /// Which match it was, from 1, of all the pattern's.
    seq: u64,
    line: String,
    captures: Vec<(String, String)>,
}

impl Sample {
    fn report(&self) -> Value {
        let captures: Map<String, Value> = self
            .captures
            .iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        json!({"group": self.group, "seq": self.seq, "line": self.line, "captures": captures})
    }
}

/// What one [`Spec`] has kept of the lines it matched.
#[derive(Debug)]
pub struct Tally {
    spec: Spec,
    count: u64,
    groups: Vec<Group>,
    index: HashMap<Vec<String>, usize>,
    /// Matches of groups past `max_groups`.
    dropped_matches: u64,
    dropped_groups: HashSet<Vec<String>>,
    dropped_groups_truncated: bool,
    samples: Vec<Sample>,
}

impl Tally {
    pub fn new(spec: Spec) -> Tally {
        Tally {
            spec,
            count: 0,
            groups: vec![],
            index: HashMap::new(),
            dropped_matches: 0,
            dropped_groups: HashSet::new(),
            dropped_groups_truncated: false,
            samples: vec![],
        }
    }

    pub fn id(&self) -> &str {
        &self.spec.id
    }

    /// Take in `line`, a console line's message text, if it matches.
    pub fn add(&mut self, line: &str) {
        let Some(caps) = self.spec.regex.captures(line) else {
            return;
        };
        self.count += 1;
        let value = |name: &str| -> Option<String> {
            if name == LINE {
                Some(cut(line, MAX_VALUE_BYTES).to_string())
            } else {
                caps.name(name)
                    .map(|m| cut(m.as_str(), MAX_VALUE_BYTES).to_string())
            }
        };
        let key: Vec<String> = self
            .spec
            .group_by
            .iter()
            .map(|g| value(g).unwrap_or_default())
            .collect();
        let group = match self.index.get(&key) {
            Some(&i) => i,
            None if self.groups.len() < self.spec.max_groups => {
                let states = self
                    .spec
                    .aggregations
                    .iter()
                    .map(|(_, a)| State::new(a))
                    .collect();
                self.groups.push(Group {
                    key: key.clone(),
                    line: cut(line, MAX_VALUE_BYTES).to_string(),
                    captures: caps_named(&self.spec.regex, &caps),
                    last: None,
                    count: 0,
                    states,
                });
                self.index.insert(key, self.groups.len() - 1);
                self.groups.len() - 1
            }
            None => {
                self.dropped_matches += 1;
                if self.dropped_groups.len() < MAX_DISTINCT {
                    self.dropped_groups.insert(key);
                } else if !self.dropped_groups.contains(&key) {
                    self.dropped_groups_truncated = true;
                }
                return;
            }
        };
        let g = &mut self.groups[group];
        g.count += 1;
        for ((_, a), state) in self.spec.aggregations.iter().zip(g.states.iter_mut()) {
            if let Some(v) = a.capture().and_then(value) {
                state.add(&v);
            }
        }
        let sample = Sample {
            group,
            seq: self.count,
            line: cut(line, MAX_VALUE_BYTES).to_string(),
            captures: caps_named(&self.spec.regex, &caps),
        };
        if self.samples.len() < self.spec.samples {
            self.samples.push(sample.clone());
        }
        self.groups[group].last = Some(sample);
    }

    /// What it kept, as data: `{"id", "count", "groups", "samples",
    /// "dropped_groups", "dropped_groups_truncated", "dropped_matches"}`.
    ///
    /// A group, in the order first matched, is `{"key": {capture: value},
    /// "line" and "captures" (its first match's), "last" (its last match, as
    /// a sample), "count", "values": {name: value}, "truncated": [name]}`. A
    /// sample is `{"group": index, "seq": which match, from 1, "line",
    /// "captures": {name: value}}`.
    pub fn report(&self) -> Value {
        let groups: Vec<Value> = self
            .groups
            .iter()
            .map(|g| {
                let key: Map<String, Value> = self
                    .spec
                    .group_by
                    .iter()
                    .cloned()
                    .zip(g.key.iter().map(|v| json!(v)))
                    .collect();
                let mut values = Map::new();
                let mut truncated = vec![];
                for ((name, _), state) in self.spec.aggregations.iter().zip(&g.states) {
                    let (v, cut) = state.report(g.count);
                    values.insert(name.clone(), v);
                    if cut {
                        truncated.push(json!(name));
                    }
                }
                let captures: Map<String, Value> = g
                    .captures
                    .iter()
                    .map(|(k, v)| (k.clone(), json!(v)))
                    .collect();
                json!({
                    "key": key,
                    "line": g.line,
                    "captures": captures,
                    "last": g.last.as_ref().map(Sample::report),
                    "count": g.count,
                    "values": values,
                    "truncated": truncated,
                })
            })
            .collect();
        let samples: Vec<Value> = self.samples.iter().map(Sample::report).collect();
        json!({
            "id": self.spec.id,
            "count": self.count,
            "groups": groups,
            "dropped_groups": self.dropped_groups.len(),
            "dropped_groups_truncated": self.dropped_groups_truncated,
            "dropped_matches": self.dropped_matches,
            "samples": samples,
        })
    }
}

fn caps_named(regex: &Regex, caps: &regex::Captures) -> Vec<(String, String)> {
    regex
        .capture_names()
        .flatten()
        .filter_map(|n| {
            caps.name(n)
                .map(|m| (n.to_string(), cut(m.as_str(), MAX_VALUE_BYTES).to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(
        pattern: &str,
        group_by: &[&str],
        aggs: Vec<(&str, Aggregation)>,
        samples: usize,
        max_groups: usize,
    ) -> Spec {
        Spec::new(
            "r".into(),
            pattern,
            group_by.iter().map(|s| s.to_string()).collect(),
            aggs.into_iter().map(|(n, a)| (n.to_string(), a)).collect(),
            samples,
            max_groups,
        )
        .unwrap()
    }

    const DEP: &str = r"target '(?P<target>[^']+)' depends on deprecated target '(?P<dep>[^']+)'";

    fn dep(t: &str, d: &str) -> String {
        format!("WARNING: x: target '{t}' depends on deprecated target '{d}': use y")
    }

    #[test]
    fn groups_by_a_capture_and_aggregates_each() {
        let mut t = Tally::new(spec(
            DEP,
            &["dep"],
            vec![
                ("n", Aggregation::Count),
                (
                    "targets",
                    Aggregation::Unique {
                        capture: "target".into(),
                        max: 10,
                    },
                ),
                (
                    "first",
                    Aggregation::First {
                        capture: LINE.into(),
                    },
                ),
            ],
            1,
            10,
        ));
        for (a, d) in [
            ("//a", "//old"),
            ("//b", "//old"),
            ("//a", "//old"),
            ("//c", "//legacy"),
        ] {
            t.add(&dep(a, d));
        }
        t.add("INFO: no match");
        let r = t.report();
        assert_eq!(r["count"], 4);
        assert_eq!(r["groups"][0]["key"]["dep"], "//old");
        assert_eq!(r["groups"][0]["count"], 3);
        assert_eq!(r["groups"][0]["values"]["n"], 3);
        assert_eq!(r["groups"][0]["values"]["targets"], json!(["//a", "//b"]));
        assert_eq!(r["groups"][0]["values"]["first"], dep("//a", "//old"));
        assert_eq!(r["groups"][1]["line"], dep("//c", "//legacy"));
        assert_eq!(r["groups"][1]["captures"]["target"], "//c");
        assert_eq!(r["groups"][1]["values"]["targets"], json!(["//c"]));
        assert_eq!(r["samples"].as_array().unwrap().len(), 1);
        assert_eq!(r["samples"][0]["captures"]["target"], "//a");
        assert_eq!(r["groups"][0]["last"]["seq"], 3);
        assert_eq!(r["groups"][0]["last"]["captures"]["target"], "//a");
    }

    #[test]
    fn every_kind_reports_what_it_kept() {
        let mut t = Tally::new(spec(
            DEP,
            &[],
            vec![
                (
                    "all",
                    Aggregation::All {
                        capture: "target".into(),
                        max: 3,
                    },
                ),
                (
                    "distinct",
                    Aggregation::CountDistinct {
                        capture: "target".into(),
                    },
                ),
                (
                    "by",
                    Aggregation::CountBy {
                        capture: "dep".into(),
                        max: 1,
                    },
                ),
                (
                    "last",
                    Aggregation::Last {
                        capture: "target".into(),
                    },
                ),
            ],
            0,
            10,
        ));
        for (a, d) in [
            ("//a", "//x"),
            ("//a", "//y"),
            ("//b", "//x"),
            ("//c", "//x"),
        ] {
            t.add(&dep(a, d));
        }
        let g = &t.report()["groups"][0];
        assert_eq!(g["values"]["all"], json!(["//a", "//a", "//b"]));
        assert_eq!(g["values"]["distinct"], 3);
        assert_eq!(g["values"]["by"], json!({"//x": 3}));
        assert_eq!(g["values"]["last"], "//c");
        assert_eq!(g["truncated"], json!(["all", "by"]));
        assert_eq!(g["key"], json!({}));
    }

    #[test]
    fn memory_stays_bounded_and_counts_exact() {
        let mut t = Tally::new(spec(
            DEP,
            &["target"],
            vec![(
                "deps",
                Aggregation::Unique {
                    capture: "dep".into(),
                    max: 2,
                },
            )],
            3,
            2,
        ));
        for i in 0..1000 {
            t.add(&dep(&format!("//t{}", i % 5), &format!("//d{i}")));
        }
        let r = t.report();
        assert_eq!(r["count"], 1000);
        assert_eq!(r["groups"].as_array().unwrap().len(), 2);
        assert_eq!(r["groups"][0]["count"], 200);
        assert_eq!(r["groups"][0]["truncated"], json!(["deps"]));
        assert_eq!(r["dropped_groups"], 3);
        assert_eq!(r["dropped_matches"], 600);
        assert_eq!(r["samples"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn a_capture_the_pattern_lacks_is_refused() {
        let err = Spec::new("r".into(), DEP, vec!["package".into()], vec![], 0, 1).unwrap_err();
        assert_eq!(
            err.to_string(),
            "r: the pattern has no named group `package` (it has: target, dep)"
        );
        assert!(
            Spec::new(
                "r".into(),
                "x",
                vec![],
                vec![(
                    "l".into(),
                    Aggregation::Last {
                        capture: LINE.into()
                    }
                )],
                0,
                1
            )
            .is_ok()
        );
    }

    #[test]
    fn a_group_named_line_is_refused() {
        let err = Spec::new(
            "r".into(),
            r"(?P<file>\S+):(?P<line>\d+)",
            vec![],
            vec![],
            0,
            1,
        )
        .unwrap_err();
        assert!(err.to_string().contains("named group `line`"), "{err}");
    }

    #[test]
    fn an_optional_capture_that_did_not_take_part_adds_nothing() {
        let mut t = Tally::new(spec(
            r"^E(?: in (?P<file>\S+))?",
            &[],
            vec![(
                "files",
                Aggregation::Unique {
                    capture: "file".into(),
                    max: 5,
                },
            )],
            0,
            1,
        ));
        t.add("E in a.go");
        t.add("E");
        let r = t.report();
        assert_eq!(r["groups"][0]["count"], 2);
        assert_eq!(r["groups"][0]["values"]["files"], json!(["a.go"]));
    }
}
