//! `bazel query`: the command line, and the streamed-proto result decoded.

use std::process::Command;

use axl_proto::blaze_query as query;
use prost::Message;

use super::stderr_detail;
use crate::runner::Runner;

#[derive(Debug, Clone)]
pub enum Target {
    // We leave environment_group out as its undocumented.
    // https://github.com/bazelbuild/bazel/issues/10849
    SourceFile(query::SourceFile),
    GeneratedFile(query::GeneratedFile),
    Rule(query::Rule),
    PackageGroup(query::PackageGroup),
}

impl TryFrom<query::Target> for Target {
    type Error = &'static str;

    fn try_from(value: query::Target) -> Result<Self, Self::Error> {
        match value.r#type() {
            query::target::Discriminator::Rule => {
                Ok(Self::Rule(value.rule.ok_or("rule field is not set.")?))
            }
            query::target::Discriminator::SourceFile => Ok(Self::SourceFile(
                value.source_file.ok_or("source_file field is not set.")?,
            )),
            query::target::Discriminator::GeneratedFile => Ok(Self::GeneratedFile(
                value
                    .generated_file
                    .ok_or("generated_file field is not set.")?,
            )),
            query::target::Discriminator::PackageGroup => Ok(Self::PackageGroup(
                value
                    .package_group
                    .ok_or("package_group field is not set.")?,
            )),
            query::target::Discriminator::EnvironmentGroup => Err("not supported"),
        }
    }
}

/// `bazel <startup flags> query <expr> <flags> --output=streamed_proto`, run
/// in `directory` when given. The caller wires up stdio.
pub fn command(
    runner: &dyn Runner,
    expr: &str,
    startup_flags: &[String],
    flags: &[String],
    directory: Option<String>,
) -> Command {
    let mut cmd = runner.command();
    cmd.args(startup_flags);
    cmd.arg("query");
    cmd.arg(expr);
    cmd.args(flags);
    cmd.arg("--output=streamed_proto");
    if let Some(dir) = directory {
        cmd.current_dir(dir);
    }
    cmd
}

/// The message for a `bazel query` that exited non-zero.
///
/// A failed query (bad expression, BUILD evaluation error, …) must not
/// be mistaken for one that legitimately matched nothing, so the message
/// carries Bazel's own `stderr` when it's available.
pub fn failure_message(expr: &str, exit_code: Option<i32>, stderr: &str) -> String {
    let detail = stderr_detail(stderr);
    format!("bazel query failed with exit code {exit_code:?}: {expr}{detail}")
}

/// The targets in a streamed-proto result. Targets of a kind that is not
/// supported are skipped; a record that fails to decode ends the result.
pub fn decode(mut buf: &[u8]) -> Vec<Target> {
    let mut targets = vec![];
    // TODO: only stop if the error is EOF
    while let Ok(target) = query::Target::decode_length_delimited(&mut buf) {
        if let Ok(target) = target.try_into() {
            targets.push(target);
        }
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_failure_message_includes_stderr_when_present() {
        let msg = failure_message("kind('x', //...)", Some(2), "ERROR: no such package\n");
        assert!(msg.contains("exit code Some(2)"), "{msg}");
        assert!(msg.contains("kind('x', //...)"), "{msg}");
        assert!(msg.contains("ERROR: no such package"), "{msg}");
    }

    #[test]
    fn query_failure_message_omits_empty_stderr() {
        let msg = failure_message("//...", Some(1), "   \n  ");
        // No dangling separator/newline when there are no diagnostics.
        assert!(msg.ends_with("//..."), "{msg}");
    }

    #[test]
    fn query_failure_message_handles_unknown_exit_code() {
        // A signal-killed child reports no exit code.
        assert!(failure_message("//...", None, "").contains("exit code None"));
    }

    #[test]
    fn decode_reads_every_length_delimited_target() {
        let rule = |name: &str| query::Target {
            r#type: query::target::Discriminator::Rule as i32,
            rule: Some(query::Rule {
                name: name.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut buf = rule("//a").encode_length_delimited_to_vec();
        buf.extend(rule("//b").encode_length_delimited_to_vec());
        let targets = decode(&buf);
        assert_eq!(targets.len(), 2);
        assert!(matches!(&targets[0], Target::Rule(r) if r.name == "//a"));
        assert!(matches!(&targets[1], Target::Rule(r) if r.name == "//b"));
    }

    #[test]
    fn decode_skips_a_target_that_cannot_be_converted() {
        let unset = query::Target {
            r#type: query::target::Discriminator::Rule as i32,
            rule: None,
            ..Default::default()
        };
        assert!(decode(&unset.encode_length_delimited_to_vec()).is_empty());
    }
}
