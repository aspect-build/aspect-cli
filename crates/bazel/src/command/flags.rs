//! Reading the effective value of a flag off a rendered command line for one
//! Bazel command: `--default_override` payloads in the order Bazel parses them
//! (see [`crate::rc::override_buckets`]), then the literal tokens, so a literal
//! wins and among overrides the more specific section does.

use crate::rc::{is_default_override, override_buckets};

/// Every option on a rendered command line for `command`, lowest precedence
/// first: the override payloads that apply to `command`, then the literal tokens.
fn option_tokens(flags: &[String], command: &str) -> Vec<String> {
    let tokens: Vec<&str> = flags.iter().map(String::as_str).collect();
    override_buckets(&tokens, command)
        .into_iter()
        .flatten()
        .chain(tokens.iter().copied().filter(|t| !is_default_override(t)))
        .map(str::to_owned)
        .collect()
}

/// The effective value of `--name=<value>` for `command`, or `None` when unset.
pub fn effective_value(flags: &[String], command: &str, name: &str) -> Option<String> {
    let eq = format!("--{name}=");
    option_tokens(flags, command)
        .into_iter()
        .filter_map(|token| token.strip_prefix(&eq).map(str::to_owned))
        .next_back()
}

/// The effective value of a boolean `--name`: `--name`, `--name=<bool>`,
/// `--noname`. `None` when unset.
pub fn effective_bool(flags: &[String], command: &str, name: &str) -> Option<bool> {
    let bare = format!("--{name}");
    let no = format!("--no{name}");
    let eq = format!("--{name}=");
    let mut value = None;
    for token in option_tokens(flags, command) {
        if token == bare {
            value = Some(true);
        } else if token == no {
            value = Some(false);
        } else if let Some(v) = token.strip_prefix(&eq) {
            value = match v.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "t" | "y" => Some(true),
                "false" | "0" | "no" | "f" | "n" => Some(false),
                _ => value,
            };
        }
    }
    value
}

/// The effective `--remote_download_outputs` mode, reading the flag and its
/// three no-value aliases. `None` when unset, which is Bazel's `toplevel`.
pub fn effective_download_outputs(flags: &[String], command: &str) -> Option<String> {
    let mut value = None;
    for token in option_tokens(flags, command) {
        match token.as_str() {
            "--remote_download_minimal" => value = Some("minimal".to_owned()),
            "--remote_download_toplevel" => value = Some("toplevel".to_owned()),
            "--remote_download_all" => value = Some("all".to_owned()),
            _ => {
                if let Some(v) = token.strip_prefix("--remote_download_outputs=") {
                    value = Some(v.to_owned());
                }
            }
        }
    }
    value
}

/// Whether a `--remote_download_outputs` mode already puts top-level outputs
/// and their runfiles on disk. Only `minimal` does not.
pub fn download_outputs_sufficient(value: Option<&str>) -> bool {
    !matches!(value, Some("minimal"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn booleans_read_every_spelling_and_the_override_form() {
        assert_eq!(
            effective_bool(&flags(&["--enable_runfiles"]), "build", "enable_runfiles"),
            Some(true)
        );
        assert_eq!(
            effective_bool(&flags(&["--noenable_runfiles"]), "build", "enable_runfiles"),
            Some(false)
        );
        assert_eq!(
            effective_bool(
                &flags(&["--enable_runfiles=no"]),
                "build",
                "enable_runfiles"
            ),
            Some(false)
        );
        assert_eq!(
            effective_bool(&flags(&[]), "build", "enable_runfiles"),
            None
        );
        assert_eq!(
            effective_bool(
                &flags(&["--default_override=0:common=--nobuild_runfile_manifests"]),
                "build",
                "build_runfile_manifests"
            ),
            Some(false)
        );
        // A literal beats an override whatever the order.
        assert_eq!(
            effective_bool(
                &flags(&[
                    "--build_runfile_manifests",
                    "--default_override=0:build=--nobuild_runfile_manifests"
                ]),
                "build",
                "build_runfile_manifests"
            ),
            Some(true)
        );
    }

    /// Among overrides the section ranks, not the token order: a `build`
    /// payload emitted ahead of a `common` one still wins, as it does in Bazel.
    #[test]
    fn a_more_specific_section_wins_whatever_the_order() {
        let f = flags(&[
            "--default_override=0:build=--noenable_runfiles",
            "--default_override=0:common=--enable_runfiles",
        ]);
        assert_eq!(effective_bool(&f, "build", "enable_runfiles"), Some(false));
        assert_eq!(effective_bool(&f, "query", "enable_runfiles"), Some(true));
    }

    /// An override for a section the command does not read is not in effect.
    #[test]
    fn an_override_for_another_command_does_not_apply() {
        let f = flags(&["--default_override=0:test=--remote_download_outputs=minimal"]);
        assert_eq!(effective_download_outputs(&f, "build"), None);
        assert_eq!(
            effective_download_outputs(&f, "test").as_deref(),
            Some("minimal")
        );
    }

    #[test]
    fn a_value_flag_reads_the_last_setting_and_a_literal_beats_an_override() {
        let name = "experimental_build_event_upload_strategy";
        assert_eq!(effective_value(&flags(&[]), "build", name), None);
        assert_eq!(
            effective_value(
                &flags(&[
                    "--experimental_build_event_upload_strategy=remote",
                    "--experimental_build_event_upload_strategy=local"
                ]),
                "build",
                name
            )
            .as_deref(),
            Some("local")
        );
        assert_eq!(
            effective_value(
                &flags(&[
                    "--experimental_build_event_upload_strategy=local",
                    "--default_override=0:common=--experimental_build_event_upload_strategy=remote"
                ]),
                "build",
                name
            )
            .as_deref(),
            Some("local"),
            "a literal flag outranks rc content"
        );
        assert_eq!(
            effective_value(
                &flags(&[
                    "--default_override=0:common=--experimental_build_event_upload_strategy=remote"
                ]),
                "build",
                name
            )
            .as_deref(),
            Some("remote")
        );
    }

    #[test]
    fn download_mode_reads_the_flag_and_its_aliases() {
        assert_eq!(effective_download_outputs(&flags(&[]), "build"), None);
        assert_eq!(
            effective_download_outputs(&flags(&["--remote_download_minimal"]), "build").as_deref(),
            Some("minimal")
        );
        assert_eq!(
            effective_download_outputs(&flags(&["--remote_download_outputs=all"]), "build")
                .as_deref(),
            Some("all")
        );
        assert_eq!(
            effective_download_outputs(
                &flags(&[
                    "--default_override=0:common=--remote_download_outputs=minimal",
                    "--remote_download_toplevel"
                ]),
                "build"
            )
            .as_deref(),
            Some("toplevel")
        );
        assert!(download_outputs_sufficient(None));
        assert!(download_outputs_sufficient(Some("toplevel")));
        assert!(download_outputs_sufficient(Some("all")));
        assert!(!download_outputs_sufficient(Some("minimal")));
    }
}
