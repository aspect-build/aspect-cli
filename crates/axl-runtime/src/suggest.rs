//! "Did you mean" suggestions for a name a user typed.
//!
//! Every name a user types is checked against one of two kinds of set:
//!
//! - **Closed sets**, where every valid name is known: features, task paths
//!   (`ctx.tasks["build"]`), args, slots, presets, `status_badges` keys. A
//!   wrong name fails, and when [`closest`] finds a near miss the error
//!   names it right after the wrong name, `; did you mean "X"?`, ahead of any
//!   list of the valid names ([`did_you_mean_then`]). A far miss keeps the
//!   error as it was.
//! - **Open sets**, where an unknown name is allowed on purpose: only the task
//!   keys of `task_presets` / `task_templates`, since a user-level
//!   `config.axl` can name tasks only some repos define. An unknown key is
//!   inert, and a near miss of a task here warns instead of failing.
//!
//! `closest` in `builtins/aspect/private/lib/text.axl` applies the same rule
//! for AXL, checked against the same table as the tests here.

/// The candidate `key` is most likely a typo of, or `None`.
///
/// `key` is a near miss when it is no candidate itself, is at least three
/// characters (so a short key isn't a couple of edits from everything), and is
/// within two edits of a candidate while changing at most half of it. The
/// fewest edits win, then the first candidate in sorted order.
pub fn closest<'a, I>(key: &str, candidates: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let key_len = key.chars().count();
    if key_len < 3 {
        return None;
    }
    let mut best: Option<(usize, &'a str)> = None;
    for candidate in candidates {
        if candidate == key {
            return None;
        }
        let len = candidate.chars().count();
        if len.abs_diff(key_len) > 2 {
            continue;
        }
        let distance = edit_distance(key, candidate);
        if distance <= 2 && 2 * distance <= len && best.is_none_or(|b| (distance, candidate) < b) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// `; did you mean "X"?` for the [`closest`] candidate to `key`, or `""`: the
/// suffix a closed-set error ends with.
pub fn did_you_mean<'a, I>(key: &str, candidates: I) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    closest(key, candidates)
        .map(|c| format!("; did you mean {c:?}?"))
        .unwrap_or_default()
}

/// The part of a closed-set error after the wrong `key`: `{sep}{rest}` as it
/// was, or, for a near miss, `; did you mean "X"? {Rest}`, so the suggestion
/// is read before a long `rest` such as the list of valid names.
pub fn did_you_mean_then<'a, I>(key: &str, candidates: I, sep: &str, rest: &str) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    match closest(key, candidates) {
        Some(c) => {
            let mut chars = rest.chars();
            let rest: String = chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default();
            format!("; did you mean {c:?}? {rest}")
                .trim_end()
                .to_owned()
        }
        None => format!("{sep}{rest}"),
    }
}

/// Levenshtein distance between `a` and `b`, in characters.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut current = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
        }
        previous = current;
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The candidates `text_test.axl` checks AXL's `closest` against.
    const CANDIDATES: [&str; 10] = [
        "build",
        "test",
        "lint",
        "format",
        "buildifier",
        "deploy",
        "worktree add",
        "worktree release",
        "add",
        "release",
    ];

    /// The cases `text_test.axl` checks AXL's `edit_distance` on.
    #[test]
    fn edit_distance_counts_characters() {
        for (a, b, expected) in [
            ("test", "test", 0),
            ("lnt", "lint", 1),
            ("tset", "test", 2),
            ("", "add", 3),
            ("tést", "test", 1),
        ] {
            assert_eq!(edit_distance(a, b), expected, "{a} - {b}");
        }
    }

    /// The cases `text_test.axl` checks AXL's `closest` on.
    #[test]
    fn a_near_miss_names_the_closest_candidate() {
        for (key, expected) in [
            ("tset", Some("test")),
            ("bulid", Some("build")),
            ("lnt", Some("lint")),
            ("worktree ad", Some("worktree add")),
            ("relase", Some("release")),
            ("buildifer", Some("buildifier")),
            ("deplyo", Some("deploy")),
            ("tost", Some("test")),
            ("test", None),
            ("add", None),
            ("worktree add", None),
            ("release-notes", None),
            ("fmt", None),
            ("abc", None),
            ("ad", None),
            ("tt", None),
        ] {
            assert_eq!(closest(key, CANDIDATES), expected, "{key}");
        }
    }

    #[test]
    fn ties_go_to_the_first_in_sorted_order() {
        assert_eq!(closest("bat", ["cat", "bar"]), Some("bar"));
        assert_eq!(closest("bat", ["bar", "cat"]), Some("bar"));
    }

    #[test]
    fn the_suffix_names_the_near_miss_only() {
        assert_eq!(
            did_you_mean("tset", CANDIDATES),
            r#"; did you mean "test"?"#
        );
        assert_eq!(did_you_mean("release-notes", CANDIDATES), "");
        assert_eq!(did_you_mean("tt", CANDIDATES), "");
    }

    #[test]
    fn the_suggestion_goes_before_the_rest() {
        let then = |key| did_you_mean_then(key, CANDIDATES, "; ", "expected one of a, b");
        assert_eq!(
            then("tset"),
            r#"; did you mean "test"? Expected one of a, b"#
        );
        assert_eq!(then("release-notes"), "; expected one of a, b");
        assert_eq!(then("tt"), "; expected one of a, b");
        assert_eq!(
            did_you_mean_then("tset", CANDIDATES, ". ", ""),
            r#"; did you mean "test"?"#
        );
    }
}
