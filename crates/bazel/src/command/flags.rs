//! Reading the effective value of a flag off a rendered command line.

/// `(section, token)` for every option on a rendered command line, reading
/// `--default_override=N:section=payload` the way [`crate::rc::last_flag_value`]
/// does, overrides first and literal tokens last so a literal wins.
pub fn option_tokens(flags: &[String]) -> Vec<String> {
    let mut overrides = Vec::new();
    let mut literals = Vec::new();
    for flag in flags {
        match flag.strip_prefix("--default_override=") {
            Some(rest) => {
                let payload = rest
                    .split_once(':')
                    .and_then(|(_, r)| r.split_once('='))
                    .map(|(_, p)| p)
                    .unwrap_or(rest);
                overrides.push(payload.to_owned());
            }
            None => literals.push(flag.clone()),
        }
    }
    overrides.extend(literals);
    overrides
}

/// The effective value of a boolean `--name`: `--name`, `--name=<bool>`,
/// `--noname`. `None` when unset.
pub fn effective_bool(flags: &[String], name: &str) -> Option<bool> {
    let bare = format!("--{name}");
    let no = format!("--no{name}");
    let eq = format!("--{name}=");
    let mut value = None;
    for token in option_tokens(flags) {
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
pub fn effective_download_outputs(flags: &[String]) -> Option<String> {
    let mut value = None;
    for token in option_tokens(flags) {
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
            effective_bool(&flags(&["--enable_runfiles"]), "enable_runfiles"),
            Some(true)
        );
        assert_eq!(
            effective_bool(&flags(&["--noenable_runfiles"]), "enable_runfiles"),
            Some(false)
        );
        assert_eq!(
            effective_bool(&flags(&["--enable_runfiles=no"]), "enable_runfiles"),
            Some(false)
        );
        assert_eq!(effective_bool(&flags(&[]), "enable_runfiles"), None);
        assert_eq!(
            effective_bool(
                &flags(&["--default_override=0:common=--nobuild_runfile_manifests"]),
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
                "build_runfile_manifests"
            ),
            Some(true)
        );
    }

    #[test]
    fn download_mode_reads_the_flag_and_its_aliases() {
        assert_eq!(effective_download_outputs(&flags(&[])), None);
        assert_eq!(
            effective_download_outputs(&flags(&["--remote_download_minimal"])).as_deref(),
            Some("minimal")
        );
        assert_eq!(
            effective_download_outputs(&flags(&["--remote_download_outputs=all"])).as_deref(),
            Some("all")
        );
        assert_eq!(
            effective_download_outputs(&flags(&[
                "--default_override=0:common=--remote_download_outputs=minimal",
                "--remote_download_toplevel"
            ]))
            .as_deref(),
            Some("toplevel")
        );
        assert!(download_outputs_sufficient(None));
        assert!(download_outputs_sufficient(Some("toplevel")));
        assert!(download_outputs_sufficient(Some("all")));
        assert!(!download_outputs_sufficient(Some("minimal")));
    }
}
