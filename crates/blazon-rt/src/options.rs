//! Bazel's command-line options as static data: the shape of the tables
//! `blazon` generates from a release's option classes (`[options]` in the
//! spec), and the lookups over them.
//!
//! A generated module has `OPTIONS: &[OptionDef]`, sorted by name, so
//! [`find`] is a binary search and nothing allocates at startup.

/// What an option's value is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Long,
    Double,
    String,
    Label,
    Path,
    Duration,
    /// A flag that takes no value.
    Void,
    /// The enum's constants as declared; Bazel's converter matches them case-insensitively.
    Enum(&'static [&'static str]),
    List(&'static Kind),
    /// A string-keyed dictionary of `key=value` assignments, with this value kind.
    Dict(&'static Kind),
    /// Accepted as a string and converted by this converter class (or, when the
    /// text starts `default converter for`, by the one for that field type).
    Opaque(&'static str),
}

/// One option as a Bazel release declares it.
#[derive(Clone, Copy, Debug)]
pub struct OptionDef {
    /// The option's name exactly as Bazel declares it (`compilation_mode`).
    pub name: &'static str,
    /// The one-character abbreviation, `'\0'` when there is none.
    pub abbrev: char,
    /// The declaring class, in Java binary form (`pkg.Outer$Inner`).
    pub class: &'static str,
    pub field: &'static str,
    pub kind: Kind,
    /// The default as Bazel writes it on a command line.
    pub default: &'static str,
    pub help: &'static str,
    pub value_help: &'static str,
    /// The documentation category (`OptionDocumentationCategory`).
    pub category: &'static str,
    pub effect_tags: &'static [&'static str],
    pub metadata_tags: &'static [&'static str],
    pub allows_multiple: bool,
    pub deprecated: bool,
    pub deprecation_warning: &'static str,
    /// Sets other flags rather than a value of its own.
    pub expansion: bool,
    pub expands_to: &'static [&'static str],
    pub implicit_requirements: &'static [&'static str],
    pub old_names: &'static [&'static str],
    /// The declaring class is a configuration fragment (`FragmentOptions`).
    pub fragment: bool,
}

/// The option called `name` (or that used to be), in a table sorted by name.
pub fn find(options: &'static [OptionDef], name: &str) -> Option<&'static OptionDef> {
    match options.binary_search_by(|o| o.name.cmp(name)) {
        Ok(i) => Some(&options[i]),
        Err(_) => options.iter().find(|o| o.old_names.contains(&name)),
    }
}

/// Names that look like what was typed: within two edits, or sharing a
/// prefix or a contained word, nearest first, at most five.
pub fn closest(options: &'static [OptionDef], name: &str) -> Vec<&'static str> {
    let mut scored: Vec<(usize, &'static str)> = Vec::new();
    for o in options {
        let d = distance(o.name, name);
        let related = o.name.starts_with(name)
            || (name.len() >= 4 && o.name.contains(name))
            || (o.name.len() >= 4 && name.contains(o.name));
        if d <= 2 || related {
            scored.push((if d <= 2 { d } else { 3 }, o.name));
        }
    }
    scored.sort();
    scored.into_iter().take(5).map(|(_, n)| n).collect()
}

/// Levenshtein distance, on bytes (option names are ASCII).
fn distance(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn def(name: &'static str, kind: Kind, old: &'static [&'static str]) -> OptionDef {
        OptionDef {
            name,
            abbrev: '\0',
            class: "x.Y",
            field: "f",
            kind,
            default: "",
            help: "",
            value_help: "",
            category: "UNCATEGORIZED",
            effect_tags: &[],
            metadata_tags: &[],
            allows_multiple: false,
            deprecated: false,
            deprecation_warning: "",
            expansion: false,
            expands_to: &[],
            implicit_requirements: &[],
            old_names: old,
            fragment: false,
        }
    }

    static TABLE: &[OptionDef] = &[
        def(
            "compilation_mode",
            Kind::Enum(&["FASTBUILD", "DBG", "OPT"]),
            &[],
        ),
        def("copt", Kind::List(&Kind::String), &[]),
        def("cpu", Kind::String, &[]),
        def("experimental_foo", Kind::Bool, &["foo"]),
        def("stamp", Kind::Bool, &[]),
    ];

    #[test]
    fn finds_by_name_and_old_name() {
        assert_eq!(find(TABLE, "cpu").map(|o| o.name), Some("cpu"));
        assert_eq!(find(TABLE, "foo").map(|o| o.name), Some("experimental_foo"));
        assert!(find(TABLE, "nope").is_none());
        assert_eq!(
            find(TABLE, "copt").map(|o| o.kind),
            Some(Kind::List(&Kind::String))
        );
    }

    #[test]
    fn suggests_close_names() {
        assert_eq!(closest(TABLE, "compilation_mod"), ["compilation_mode"]);
        assert!(closest(TABLE, "stmp").contains(&"stamp"));
        assert!(closest(TABLE, "mode").contains(&"compilation_mode"));
        assert!(closest(TABLE, "zzzzzz").is_empty());
    }
}
