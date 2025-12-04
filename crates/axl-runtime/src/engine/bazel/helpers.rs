pub fn join_strings(items: &[impl AsRef<str>], sep: &str) -> String {
    if items.is_empty() {
        return String::new();
    }
    items
        .iter()
        .map(|s| s.as_ref())
        .collect::<Vec<_>>()
        .join(sep)
}

pub fn format_bazel_command(
    startup_flags: &Vec<String>,
    verb: &str,
    flags: &Vec<String>,
    targets: &Vec<String>,
) -> String {
    let startup_str = join_strings(&startup_flags, " ");
    let flags_str = join_strings(&flags, " ");
    let targets_str = join_strings(&targets, " ");

    let mut parts: Vec<String> = Vec::new();
    parts.push("bazel".to_string());
    if !startup_str.is_empty() {
        parts.push(startup_str);
    }
    parts.push(verb.to_string());
    if !flags_str.is_empty() {
        parts.push(flags_str);
    }
    parts.push("--".to_string());
    if !targets_str.is_empty() {
        parts.push(targets_str);
    }

    join_strings(&parts, " ")
}
