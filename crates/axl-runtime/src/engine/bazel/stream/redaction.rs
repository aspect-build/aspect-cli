//! Secret redaction for Build Event Protocol events.
//!
//! Bazel's BEP stream faithfully echoes back every command-line flag the user
//! (or CI) passed, including values that frequently carry credentials:
//! `--remote_header=Authorization: Bearer <token>`,
//! `--bes_backend=grpcs://user:password@host:port`,
//! `--action_env=DEPLOY_TOKEN=<secret>`, etc. Without redaction, those secrets
//! flow downstream to every BES sink — the gRPC forwarder, file dumps uploaded
//! as artifacts, the AXL `build_events()` iterator. We don't want that.
//!
//! This module rewrites events in place before fan-out, so every downstream
//! consumer sees a scrubbed stream. Secrets never leave the process.
//!
//! The rule set mirrors the AXL-side redaction in `bazel_results.axl`
//! (`_redact_arg` / `_strip_url_creds`). Any change here should be reflected
//! there, or vice versa — we should move both to share a single source of
//! truth once the API surface settles.
//!
//! User-configurable allowlist: the user can pass
//! `--build_metadata=ALLOW_ENV=MY_VAR,OTHER_*` on the Bazel command to extend
//! (not replace) the default allowlist. We extract that flag from the same
//! event we're redacting — command-line events carry the flag in their own
//! args list, so there's no ordering dependency on BuildMetadata arrival.
//! Glob patterns (`*`, `?`) are supported, matching the AXL semantics.

use axl_proto::build_event_stream::BuildEvent;
use axl_proto::build_event_stream::build_event::Payload;
use axl_proto::command_line::command_line_section::SectionType;
use axl_proto::command_line::{CommandLine, Option as CmdOption};
use std::borrow::Cow;

pub const REDACTED: &str = "<REDACTED>";

/// Bazel flag names whose values carry gRPC/HTTP headers — always redacted
/// since headers routinely carry bearer tokens, API keys, cookies, etc.
const HEADER_OPTION_NAMES: &[&str] = &[
    "remote_header",
    "remote_cache_header",
    "remote_exec_header",
    "remote_downloader_header",
    "bes_header",
];

/// Bazel flag names of the `--foo=KEY=VALUE` env-passthrough form. The VALUE
/// portion is redacted unless the KEY matches the ALLOW_ENV allowlist.
const ENV_VAR_OPTION_NAMES: &[&str] = &[
    "action_env",
    "client_env",
    "host_action_env",
    "repo_env",
    "test_env",
];

/// Env var names whose value is expected to be a git repository URL. The URL
/// itself is informative, but any user:password prefix gets scrubbed.
const KNOWN_GIT_REPO_URL_KEYS: &[&str] = &[
    "REPO_URL",
    "GIT_URL",
    "TRAVIS_REPO_SLUG",
    "BUILDKITE_REPO",
    "GIT_REPOSITORY_URL",
    "CIRCLE_REPOSITORY_URL",
    "GITHUB_REPOSITORY",
    "CI_REPOSITORY_URL",
];

/// Metadata key used by some CI runners to store the original command line
/// as a JSON-encoded list of strings. When present, we decode, redact each
/// flag, and re-encode — otherwise any secrets nested inside
/// (`--remote_header=...`, `--action_env=TOKEN=...`) slip through on the
/// BuildMetadata event untouched.
const EXPLICIT_COMMAND_LINE_KEY: &str = "EXPLICIT_COMMAND_LINE";

/// Curated CI/VCS identifiers that are publicly safe to surface (commit SHA,
/// branch, PR number, run ID, repo URL — the kind of info that typically ends
/// up in the commit message or CI job name anyway). Values of env vars we
/// don't recognize are redacted by default.
const DEFAULT_ALLOW_ENV: &[&str] = &[
    // Who ran the build.
    "USER",
    "GITHUB_ACTOR",
    "BUILDKITE_BUILD_CREATOR",
    "GITLAB_USER_NAME",
    "CIRCLE_USERNAME",
    // Repo URLs (stripped of credentials separately).
    "GITHUB_REPOSITORY",
    "BUILDKITE_REPO",
    "TRAVIS_REPO_SLUG",
    "GIT_URL",
    "GIT_REPOSITORY_URL",
    "CI_REPOSITORY_URL",
    "REPO_URL",
    "CIRCLE_REPOSITORY_URL",
    // Commit SHAs.
    "GITHUB_SHA",
    "CIRCLE_SHA1",
    "BUILDKITE_COMMIT",
    "TRAVIS_COMMIT",
    "BITRISE_GIT_COMMIT",
    "GIT_COMMIT",
    "VOLATILE_GIT_COMMIT",
    "CI_COMMIT_SHA",
    "COMMIT_SHA",
    // Run identifiers.
    "GITHUB_RUN_ID",
    "BUILDKITE_BUILD_URL",
    "BUILDKITE_JOB_ID",
    // Branches / refs.
    "GITHUB_HEAD_REF",
    "GITHUB_REF",
    "CIRCLE_BRANCH",
    "BUILDKITE_BRANCH",
    "TRAVIS_BRANCH",
    "BITRISE_GIT_BRANCH",
    "GIT_BRANCH",
    "CI_COMMIT_BRANCH",
    "CI_MERGE_REQUEST_SOURCE_BRANCH_NAME",
    // Generic CI flags.
    "CI",
    "CI_RUNNER",
];

/// Values that are always safe to surface regardless of allowlist: no-op
/// markers and booleans that never carry secrets.
const SAFE_ENV_VALUES: &[&str] = &[
    "", "0", "1", "true", "false", "True", "False", "TRUE", "FALSE",
];

/// Case-insensitive glob match with `*` and `?` support. Iterative with
/// backtracking — no regex dependency. Matches the AXL `_glob_match`
/// semantics so a pattern that works in AXL works here too.
fn glob_match(pattern: &str, name: &str) -> bool {
    let pat: Vec<char> = pattern.to_lowercase().chars().collect();
    let nm: Vec<char> = name.to_lowercase().chars().collect();
    let mut pi = 0usize;
    let mut ni = 0usize;
    let mut star_pi: Option<usize> = None;
    let mut star_ni = 0usize;
    loop {
        if ni < nm.len() && pi < pat.len() && (pat[pi] == '?' || pat[pi] == nm[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            star_pi = Some(pi);
            star_ni = ni;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ni += 1;
            ni = star_ni;
        } else {
            return false;
        }
        if ni >= nm.len() {
            // Eat any trailing '*'s in the pattern.
            while pi < pat.len() && pat[pi] == '*' {
                pi += 1;
            }
            return pi == pat.len();
        }
    }
}

/// True if `name` matches any allowlist entry (exact or glob).
fn is_allowed_env(name: &str, allow_env: &[&str]) -> bool {
    allow_env.iter().any(|pat| {
        if pat.contains('*') || pat.contains('?') {
            glob_match(pat, name)
        } else {
            pat.eq_ignore_ascii_case(name)
        }
    })
}

/// Extract the user's `--build_metadata=ALLOW_ENV=...` patterns from any flag
/// string that looks like `--build_metadata=ALLOW_ENV=A,B,C_*`. Case-insensitive
/// on the key portion because Bazel preserves user casing.
fn parse_allow_env_flag(flag: &str) -> Option<Vec<String>> {
    let rest = flag.strip_prefix("--build_metadata=")?;
    let (key, value) = rest.split_once('=')?;
    if !key.eq_ignore_ascii_case("ALLOW_ENV") {
        return None;
    }
    Some(
        value
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

/// Walk an iterator of flag strings and return every ALLOW_ENV pattern the
/// user supplied (later --build_metadata=ALLOW_ENV=... flags accumulate rather
/// than replace, matching Bazel's own repeated-flag semantics).
fn collect_allow_env<'a>(flags: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out = Vec::new();
    for f in flags {
        if let Some(v) = parse_allow_env_flag(f) {
            out.extend(v);
        }
    }
    out
}

/// Build the effective allowlist for a single event: default + user-provided.
/// The resulting Vec is owned but short-lived (one allocation per redacted
/// event; typically ~1-2 user patterns on top of the default set).
fn effective_allow_env<'a>(user_patterns: &'a [String]) -> Vec<&'a str> {
    let mut out: Vec<&str> = DEFAULT_ALLOW_ENV.to_vec();
    out.extend(user_patterns.iter().map(|s| s.as_str()));
    out
}

/// Rewrite `event` in place with credentials/secrets redacted. Returns true
/// if any field was modified (caller can skip re-encode on false).
///
/// Only a small set of event payload types carry sensitive values, so the
/// common case is a cheap payload-kind check that returns false immediately.
pub fn redact_event(event: &mut BuildEvent) -> bool {
    let Some(payload) = event.payload.as_mut() else {
        return false;
    };
    match payload {
        Payload::Started(started) => {
            // options_description is a free-form shell-style summary of non-
            // default options, including quoted values with embedded spaces
            // (`--repo_env='JAVA_HOME=../bazel_tools/jdk'`). Token-level
            // redaction here would need a real shell-quote-aware tokenizer,
            // which is brittle and error-prone — a miss leaks a secret.
            //
            // StructuredCommandLine and OptionsParsed carry the same flags in
            // structured form, which we do redact precisely. No consumer in
            // this codebase reads options_description, so clearing it is the
            // simplest safe choice.
            if started.options_description.is_empty() {
                false
            } else {
                started.options_description = String::new();
                true
            }
        }
        Payload::UnstructuredCommandLine(ucl) => {
            let user_patterns = collect_allow_env(ucl.args.iter().map(|s| s.as_str()));
            let allow_env = effective_allow_env(&user_patterns);
            redact_string_vec(&mut ucl.args, &allow_env)
        }
        Payload::StructuredCommandLine(cl) => {
            let user_patterns = collect_allow_env_from_command_line(cl);
            let allow_env = effective_allow_env(&user_patterns);
            redact_command_line(cl, &allow_env)
        }
        Payload::OptionsParsed(op) => {
            // ALLOW_ENV can appear in any of the four lists; collect from all.
            let user_patterns: Vec<String> = {
                let mut v = collect_allow_env(op.startup_options.iter().map(|s| s.as_str()));
                v.extend(collect_allow_env(
                    op.explicit_startup_options.iter().map(|s| s.as_str()),
                ));
                v.extend(collect_allow_env(op.cmd_line.iter().map(|s| s.as_str())));
                v.extend(collect_allow_env(
                    op.explicit_cmd_line.iter().map(|s| s.as_str()),
                ));
                v
            };
            let allow_env = effective_allow_env(&user_patterns);
            let a = redact_string_vec(&mut op.startup_options, &allow_env);
            let b = redact_string_vec(&mut op.explicit_startup_options, &allow_env);
            let c = redact_string_vec(&mut op.cmd_line, &allow_env);
            let d = redact_string_vec(&mut op.explicit_cmd_line, &allow_env);
            a || b || c || d
        }
        Payload::WorkspaceStatus(ws) => {
            // Workspace status is user-provided via the workspace_status_command.
            // Keys (usually BUILD_*, STABLE_*) aren't secrets; values typically
            // carry git info. Strip URL credentials but leave values intact —
            // redacting them entirely would break the Aspect Web UI which reads
            // these for display.
            let mut modified = false;
            for item in ws.item.iter_mut() {
                if let Cow::Owned(s) = strip_url_creds(&item.value) {
                    item.value = s;
                    modified = true;
                }
            }
            modified
        }
        Payload::BuildMetadata(bm) => {
            // User-provided --build_metadata=KEY=VALUE entries. Two passes:
            //
            //   1. Only scrub URL creds from keys known to carry git URLs.
            //      Doing it unconditionally risks false positives on arbitrary
            //      metadata values that happen to contain `://...@`.
            //   2. Special-case EXPLICIT_COMMAND_LINE: some CI runners stuff
            //      the original command line into metadata as a JSON-encoded
            //      list of strings. Decode, redact each flag, re-encode —
            //      otherwise nested secrets (--remote_header=, --action_env=)
            //      slip through untouched.
            //
            // Users who put a bare secret in `--build_metadata=X=secret`
            // accepted that risk by passing it explicitly; we don't have
            // enough signal to redact arbitrary values.
            let mut modified = false;
            for key in KNOWN_GIT_REPO_URL_KEYS {
                if let Some(value) = bm.metadata.get_mut(*key) {
                    if let Cow::Owned(s) = strip_url_creds(value) {
                        *value = s;
                        modified = true;
                    }
                }
            }
            if let Some(raw) = bm.metadata.get_mut(EXPLICIT_COMMAND_LINE_KEY) {
                // Parse ALLOW_ENV from the embedded command line first so user
                // overrides apply to this payload (same self-describing pattern
                // as the command-line events).
                if let Ok(mut tokens) = serde_json::from_str::<Vec<String>>(raw) {
                    let user_patterns = collect_allow_env(tokens.iter().map(|s| s.as_str()));
                    let allow_env = effective_allow_env(&user_patterns);
                    let mut tokens_modified = false;
                    for t in tokens.iter_mut() {
                        if let Cow::Owned(new) = redact_flag(t, &allow_env) {
                            *t = new;
                            tokens_modified = true;
                        }
                    }
                    if tokens_modified {
                        if let Ok(encoded) = serde_json::to_string(&tokens) {
                            *raw = encoded;
                            modified = true;
                        }
                    }
                }
            }
            modified
        }
        _ => false,
    }
}

/// Extract ALLOW_ENV patterns from a structured command line by walking the
/// `--build_metadata` options nested inside OptionList sections.
fn collect_allow_env_from_command_line(cl: &CommandLine) -> Vec<String> {
    let mut out = Vec::new();
    for section in &cl.sections {
        let Some(section_type) = section.section_type.as_ref() else {
            continue;
        };
        match section_type {
            SectionType::ChunkList(cl) => {
                out.extend(collect_allow_env(cl.chunk.iter().map(|s| s.as_str())));
            }
            SectionType::OptionList(ol) => {
                for opt in &ol.option {
                    if let Some(v) = parse_allow_env_flag(&opt.combined_form) {
                        out.extend(v);
                    }
                    // Also check option_name/option_value in case combined_form
                    // is empty (e.g. canonical form).
                    if opt.option_name.eq_ignore_ascii_case("build_metadata") {
                        if let Some((key, val)) = opt.option_value.split_once('=') {
                            if key.eq_ignore_ascii_case("ALLOW_ENV") {
                                out.extend(
                                    val.split(',')
                                        .map(|s| s.trim().to_string())
                                        .filter(|s| !s.is_empty()),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

fn redact_string_vec(v: &mut Vec<String>, allow_env: &[&str]) -> bool {
    let mut modified = false;
    for s in v.iter_mut() {
        if let Cow::Owned(new) = redact_flag(s, allow_env) {
            *s = new;
            modified = true;
        }
    }
    modified
}

fn redact_command_line(cl: &mut CommandLine, allow_env: &[&str]) -> bool {
    let mut modified = false;
    for section in cl.sections.iter_mut() {
        let Some(section_type) = section.section_type.as_mut() else {
            continue;
        };
        match section_type {
            SectionType::ChunkList(cl) => {
                if redact_string_vec(&mut cl.chunk, allow_env) {
                    modified = true;
                }
            }
            SectionType::OptionList(ol) => {
                for opt in ol.option.iter_mut() {
                    if redact_option(opt, allow_env) {
                        modified = true;
                    }
                }
            }
        }
    }
    modified
}

fn redact_option(opt: &mut CmdOption, allow_env: &[&str]) -> bool {
    let name = opt.option_name.as_str();
    let new_value = redact_option_value(name, &opt.option_value, allow_env);
    let new_combined = redact_flag(&opt.combined_form, allow_env);

    let mut modified = false;
    if let Cow::Owned(s) = new_value {
        opt.option_value = s;
        modified = true;
    }
    if let Cow::Owned(s) = new_combined {
        opt.combined_form = s;
        modified = true;
    }
    modified
}

/// Scrub a single flag string (`--name=value` / `--name value` / positional).
///
/// - Name-only flags (no `=`): leave untouched.
/// - `--header-like=VALUE`: redact entire VALUE.
/// - `--env-like=KEY=VALUE`: redact VALUE unless KEY is on `allow_env` or
///   is a known-safe literal; strip URL creds when KEY is a known git URL var.
/// - Other `--name=value`: strip URL creds from VALUE.
pub fn redact_flag<'a>(arg: &'a str, allow_env: &[&str]) -> Cow<'a, str> {
    // Positional / non-flag: only URL-creds scrub.
    if !arg.starts_with("--") {
        return strip_url_creds(arg);
    }
    let Some(eq) = arg.find('=') else {
        // `--flag` with no value — nothing to redact.
        return Cow::Borrowed(arg);
    };
    let name = &arg[2..eq];
    let value = &arg[eq + 1..];

    if HEADER_OPTION_NAMES.contains(&name) {
        return Cow::Owned(format!("--{}={}", name, REDACTED));
    }
    if ENV_VAR_OPTION_NAMES.contains(&name) {
        let new_value = redact_env_value(value, allow_env);
        return match new_value {
            Cow::Owned(v) => Cow::Owned(format!("--{}={}", name, v)),
            Cow::Borrowed(_) => Cow::Borrowed(arg),
        };
    }
    // Other flags: strip URL creds from the value.
    match strip_url_creds(value) {
        Cow::Owned(v) => Cow::Owned(format!("--{}={}", name, v)),
        Cow::Borrowed(_) => Cow::Borrowed(arg),
    }
}

/// The raw value portion of a structured command-line option. Uses the same
/// rules as `redact_flag` but starting from the `option_value` directly, so
/// we don't have to reconstruct `--name=value`.
fn redact_option_value<'a>(name: &str, value: &'a str, allow_env: &[&str]) -> Cow<'a, str> {
    if HEADER_OPTION_NAMES.contains(&name) {
        return Cow::Owned(REDACTED.to_string());
    }
    if ENV_VAR_OPTION_NAMES.contains(&name) {
        return redact_env_value(value, allow_env);
    }
    strip_url_creds(value)
}

/// For `KEY=VALUE` env-passthrough values: redact VALUE unless KEY is on the
/// allowlist, or the value is a known safe literal, or KEY is a known git-URL
/// var (in which case we just strip URL creds).
fn redact_env_value<'a>(value: &'a str, allow_env: &[&str]) -> Cow<'a, str> {
    let Some(eq) = value.find('=') else {
        // No KEY=VALUE shape — just strip URL creds from the whole thing.
        return strip_url_creds(value);
    };
    let key = &value[..eq];
    let val = &value[eq + 1..];

    if SAFE_ENV_VALUES.contains(&val) {
        return Cow::Borrowed(value);
    }
    if is_allowed_env(key, allow_env) {
        // Allowlisted — keep the value, but still scrub URL creds from it.
        return match strip_url_creds(val) {
            Cow::Owned(v) => Cow::Owned(format!("{}={}", key, v)),
            Cow::Borrowed(_) => Cow::Borrowed(value),
        };
    }
    if KNOWN_GIT_REPO_URL_KEYS.contains(&key) {
        return match strip_url_creds(val) {
            Cow::Owned(v) => Cow::Owned(format!("{}={}", key, v)),
            Cow::Borrowed(_) => Cow::Borrowed(value),
        };
    }
    Cow::Owned(format!("{}={}", key, REDACTED))
}

/// Replace `scheme://user:password@host/...` with `scheme://<REDACTED>@host/...`.
/// No-op when the input doesn't contain `://` or the authority has no `@`.
pub fn strip_url_creds(s: &str) -> Cow<'_, str> {
    let Some(scheme_end) = s.find("://") else {
        return Cow::Borrowed(s);
    };
    // Look for `user[:pass]@` between `scheme://` and the next `/` (authority section).
    let authority_start = scheme_end + 3;
    let rest = &s[authority_start..];
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let Some(at) = authority.find('@') else {
        return Cow::Borrowed(s);
    };
    // Reconstruct with creds replaced.
    let mut out = String::with_capacity(s.len());
    out.push_str(&s[..authority_start]);
    out.push_str(REDACTED);
    out.push_str(&rest[at..]); // includes the `@` and everything after
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axl_proto::build_event_stream::{
        BuildEvent as ProtoBuildEvent, BuildMetadata as ProtoBuildMetadata,
        BuildStarted as ProtoBuildStarted, UnstructuredCommandLine as ProtoUnstructured,
    };
    use prost::Message;

    #[test]
    fn event_roundtrip_unstructured_command_line_redacts_and_reencodes() {
        let mut ev = ProtoBuildEvent {
            payload: Some(Payload::UnstructuredCommandLine(ProtoUnstructured {
                args: vec![
                    "--remote_header=Authorization: Bearer abc".to_string(),
                    "--config=ci".to_string(),
                    "--bes_backend=grpcs://alice:swordfish@bes.example.com:443".to_string(),
                ],
            })),
            ..Default::default()
        };
        let modified = redact_event(&mut ev);
        assert!(modified);
        // Re-encode + decode the redacted event; assert the decoded form matches.
        let mut buf = Vec::new();
        ev.encode(&mut buf).unwrap();
        let decoded = ProtoBuildEvent::decode(&buf[..]).unwrap();
        let Some(Payload::UnstructuredCommandLine(ucl)) = decoded.payload else {
            panic!("unexpected payload");
        };
        assert_eq!(
            ucl.args,
            vec![
                "--remote_header=<REDACTED>".to_string(),
                "--config=ci".to_string(),
                "--bes_backend=grpcs://<REDACTED>@bes.example.com:443".to_string(),
            ]
        );
    }

    #[test]
    fn event_build_metadata_strips_url_creds_only_on_known_git_keys() {
        let mut bm_map = std::collections::HashMap::new();
        bm_map.insert("COMMIT_SHA".to_string(), "abc123".to_string());
        bm_map.insert(
            "REPO_URL".to_string(),
            "https://ci:token@github.com/acme/repo".to_string(),
        );
        // Non-git-URL key with a value that looks URL-ish — should be left
        // alone so we don't mangle unrelated metadata.
        bm_map.insert(
            "WEIRD_KEY".to_string(),
            "not-really://a:b@url.example/path".to_string(),
        );
        let mut ev = ProtoBuildEvent {
            payload: Some(Payload::BuildMetadata(ProtoBuildMetadata {
                metadata: bm_map,
            })),
            ..Default::default()
        };
        let modified = redact_event(&mut ev);
        assert!(modified);
        let Some(Payload::BuildMetadata(bm)) = &ev.payload else {
            panic!("unexpected payload");
        };
        // COMMIT_SHA preserved verbatim.
        assert_eq!(bm.metadata.get("COMMIT_SHA").unwrap(), "abc123");
        // REPO_URL has user:pass stripped (known git-URL key).
        assert_eq!(
            bm.metadata.get("REPO_URL").unwrap(),
            "https://<REDACTED>@github.com/acme/repo"
        );
        // Unknown key left alone.
        assert_eq!(
            bm.metadata.get("WEIRD_KEY").unwrap(),
            "not-really://a:b@url.example/path"
        );
    }

    #[test]
    fn event_build_metadata_redacts_explicit_command_line() {
        // A CI runner stashes the original command line as JSON in metadata.
        // Secrets nested inside need to be redacted just like they are in the
        // top-level command-line events.
        let json = r#"["--config=ci","--remote_header=Authorization: Bearer sekrit","--action_env=API_TOKEN=xyz","--action_env=GITHUB_SHA=abc"]"#;
        let mut bm_map = std::collections::HashMap::new();
        bm_map.insert(EXPLICIT_COMMAND_LINE_KEY.to_string(), json.to_string());
        let mut ev = ProtoBuildEvent {
            payload: Some(Payload::BuildMetadata(ProtoBuildMetadata {
                metadata: bm_map,
            })),
            ..Default::default()
        };
        let modified = redact_event(&mut ev);
        assert!(modified);
        let Some(Payload::BuildMetadata(bm)) = &ev.payload else {
            panic!("unexpected payload");
        };
        let redacted: Vec<String> =
            serde_json::from_str(bm.metadata.get(EXPLICIT_COMMAND_LINE_KEY).unwrap()).unwrap();
        assert_eq!(
            redacted,
            vec![
                "--config=ci".to_string(),
                "--remote_header=<REDACTED>".to_string(),
                "--action_env=API_TOKEN=<REDACTED>".to_string(),
                "--action_env=GITHUB_SHA=abc".to_string(), // allowlisted
            ]
        );
    }

    #[test]
    fn event_build_metadata_explicit_command_line_honors_user_allow_env() {
        // ALLOW_ENV embedded in the explicit command line itself should be
        // extracted and applied to that same list.
        let json = r#"["--build_metadata=ALLOW_ENV=DEPLOY_ENV","--action_env=DEPLOY_ENV=prod","--action_env=OTHER=secret"]"#;
        let mut bm_map = std::collections::HashMap::new();
        bm_map.insert(EXPLICIT_COMMAND_LINE_KEY.to_string(), json.to_string());
        let mut ev = ProtoBuildEvent {
            payload: Some(Payload::BuildMetadata(ProtoBuildMetadata {
                metadata: bm_map,
            })),
            ..Default::default()
        };
        redact_event(&mut ev);
        let Some(Payload::BuildMetadata(bm)) = &ev.payload else {
            panic!("unexpected payload");
        };
        let redacted: Vec<String> =
            serde_json::from_str(bm.metadata.get(EXPLICIT_COMMAND_LINE_KEY).unwrap()).unwrap();
        assert_eq!(redacted[1], "--action_env=DEPLOY_ENV=prod");
        assert_eq!(redacted[2], "--action_env=OTHER=<REDACTED>");
    }

    #[test]
    fn event_started_clears_options_description() {
        // options_description contains a quoted-value flag — we don't even try
        // to tokenize, we just clear. The structured command-line events carry
        // the same info in redacted form.
        let mut ev = ProtoBuildEvent {
            payload: Some(Payload::Started(ProtoBuildStarted {
                options_description: "--remote_header='Authorization: Bearer s3kret' --config=ci"
                    .to_string(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let modified = redact_event(&mut ev);
        assert!(modified);
        let Some(Payload::Started(bs)) = &ev.payload else {
            panic!("unexpected payload");
        };
        assert_eq!(bs.options_description, "");
    }

    #[test]
    fn event_started_empty_options_description_is_not_modified() {
        let mut ev = ProtoBuildEvent {
            payload: Some(Payload::Started(ProtoBuildStarted::default())),
            ..Default::default()
        };
        assert!(!redact_event(&mut ev));
    }

    #[test]
    fn event_without_sensitive_payload_is_not_modified() {
        let mut ev = ProtoBuildEvent::default();
        assert!(!redact_event(&mut ev));
    }

    #[test]
    fn strips_url_creds_with_user_and_password() {
        assert_eq!(
            strip_url_creds("grpcs://alice:swordfish@bes.example.com:443"),
            "grpcs://<REDACTED>@bes.example.com:443"
        );
    }

    #[test]
    fn strips_url_creds_user_only() {
        assert_eq!(
            strip_url_creds("https://token@github.com/acme/repo.git"),
            "https://<REDACTED>@github.com/acme/repo.git"
        );
    }

    #[test]
    fn leaves_url_without_creds_untouched() {
        let s = "grpcs://bes.example.com:443";
        assert!(matches!(strip_url_creds(s), Cow::Borrowed(_)));
        assert_eq!(strip_url_creds(s), s);
    }

    #[test]
    fn leaves_non_url_strings_untouched() {
        let s = "hello world";
        assert!(matches!(strip_url_creds(s), Cow::Borrowed(_)));
    }

    fn default_allow() -> Vec<&'static str> {
        DEFAULT_ALLOW_ENV.to_vec()
    }

    #[test]
    fn redacts_header_flag() {
        assert_eq!(
            redact_flag(
                "--remote_header=Authorization: Bearer abc",
                &default_allow()
            ),
            "--remote_header=<REDACTED>"
        );
    }

    #[test]
    fn redacts_env_passthrough_unknown_key() {
        assert_eq!(
            redact_flag("--client_env=GITHUB_TOKEN=ghp_deadbeef", &default_allow()),
            "--client_env=GITHUB_TOKEN=<REDACTED>"
        );
    }

    #[test]
    fn allows_env_passthrough_known_safe() {
        // CI=true is safe literal
        let s = "--action_env=CI=true";
        assert_eq!(redact_flag(s, &default_allow()), s);
    }

    #[test]
    fn allows_env_passthrough_allowlisted_key() {
        let s = "--action_env=GITHUB_SHA=abc123def";
        assert_eq!(redact_flag(s, &default_allow()), s);
    }

    #[test]
    fn strips_url_creds_in_env_passthrough_repo_url() {
        assert_eq!(
            redact_flag(
                "--repo_env=GIT_URL=https://user:token@github.com/acme/repo",
                &default_allow()
            ),
            "--repo_env=GIT_URL=https://<REDACTED>@github.com/acme/repo"
        );
    }

    #[test]
    fn strips_url_creds_in_generic_flag() {
        assert_eq!(
            redact_flag(
                "--bes_backend=grpcs://alice:swordfish@bes.example.com:443",
                &default_allow()
            ),
            "--bes_backend=grpcs://<REDACTED>@bes.example.com:443"
        );
    }

    #[test]
    fn leaves_benign_flag_untouched() {
        let s = "--config=ci";
        assert!(matches!(redact_flag(s, &default_allow()), Cow::Borrowed(_)));
    }

    #[test]
    fn leaves_positional_target_untouched() {
        let s = "//foo:bar";
        assert!(matches!(redact_flag(s, &default_allow()), Cow::Borrowed(_)));
    }

    // --- User-configurable ALLOW_ENV ---------------------------------------

    #[test]
    fn glob_match_basics() {
        assert!(glob_match("GITHUB_*", "GITHUB_SHA"));
        assert!(glob_match("GITHUB_*", "github_actor")); // case-insensitive
        assert!(glob_match("*_TOKEN", "MY_TOKEN"));
        assert!(glob_match("A?C", "abc"));
        assert!(!glob_match("GITHUB_*", "CIRCLE_SHA"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn parse_allow_env_flag_extracts_patterns() {
        assert_eq!(
            parse_allow_env_flag("--build_metadata=ALLOW_ENV=MY_VAR,OTHER_*"),
            Some(vec!["MY_VAR".to_string(), "OTHER_*".to_string()])
        );
        assert_eq!(
            parse_allow_env_flag("--build_metadata=COMMIT_SHA=abc"),
            None
        );
        assert_eq!(parse_allow_env_flag("--config=ci"), None);
    }

    #[test]
    fn user_allow_env_keeps_custom_var_visible() {
        // Without user override, DEPLOY_ENV would be redacted.
        let baseline = redact_flag("--action_env=DEPLOY_ENV=staging", &default_allow());
        assert_eq!(baseline, "--action_env=DEPLOY_ENV=<REDACTED>");

        // Event is self-describing — we redact using the ALLOW_ENV from the
        // same event. Simulate what redact_event does with UnstructuredCommandLine.
        let mut args = vec![
            "--build_metadata=ALLOW_ENV=DEPLOY_ENV,MY_*".to_string(),
            "--action_env=DEPLOY_ENV=staging".to_string(),
            "--action_env=MY_REGION=us-west-2".to_string(),
            "--action_env=SECRET_TOKEN=sk_abc".to_string(),
        ];
        let user_patterns = collect_allow_env(args.iter().map(|s| s.as_str()));
        assert_eq!(user_patterns, vec!["DEPLOY_ENV", "MY_*"]);
        let allow_env = effective_allow_env(&user_patterns);
        let modified = redact_string_vec(&mut args, &allow_env);
        assert!(modified);
        assert_eq!(args[1], "--action_env=DEPLOY_ENV=staging"); // kept
        assert_eq!(args[2], "--action_env=MY_REGION=us-west-2"); // kept via glob
        assert_eq!(args[3], "--action_env=SECRET_TOKEN=<REDACTED>"); // redacted
    }

    #[test]
    fn multiple_allow_env_flags_accumulate() {
        let args = vec![
            "--build_metadata=ALLOW_ENV=A,B",
            "--build_metadata=ALLOW_ENV=C",
        ];
        let patterns = collect_allow_env(args.iter().copied());
        assert_eq!(patterns, vec!["A", "B", "C"]);
    }

    #[test]
    fn event_honors_user_allow_env() {
        let mut ev = ProtoBuildEvent {
            payload: Some(Payload::UnstructuredCommandLine(ProtoUnstructured {
                args: vec![
                    "--build_metadata=ALLOW_ENV=DEPLOY_ENV".to_string(),
                    "--action_env=DEPLOY_ENV=prod".to_string(),
                    "--action_env=MY_SECRET=xyz".to_string(),
                ],
            })),
            ..Default::default()
        };
        let modified = redact_event(&mut ev);
        assert!(modified);
        let Some(Payload::UnstructuredCommandLine(ucl)) = &ev.payload else {
            panic!("unexpected payload");
        };
        assert_eq!(ucl.args[1], "--action_env=DEPLOY_ENV=prod"); // kept via user allowlist
        assert_eq!(ucl.args[2], "--action_env=MY_SECRET=<REDACTED>"); // redacted
    }
}
