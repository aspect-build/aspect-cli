use std::fmt::Debug;

use allocative::Allocative;
use derive_more::Display;
use dupe::Dupe;
use starlark::StarlarkResultExt;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values;
use starlark::values::NoSerialize;
use starlark::values::ProvidesStaticType;
use starlark::values::UnpackValue;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use std::sync::Arc;

use super::stream;
use crate::color::StreamColor;
use crate::engine::cancellation::Signals;

#[derive(Debug, Display, Clone, ProvidesStaticType, NoSerialize, Allocative)]
#[display("<std.io.Stdio>")]
pub struct Stdio {
    pub stdout: stream::Writable,
    pub stderr: stream::Writable,
    pub stdin: stream::Readable,
}

impl<'v> UnpackValue<'v> for Stdio {
    type Error = anyhow::Error;

    fn unpack_value_impl(value: values::Value<'v>) -> Result<Option<Self>, Self::Error> {
        let v = value.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(Some(v.clone()))
    }
}

impl Stdio {
    /// The process's streams, stdout and stderr reporting `color` as their
    /// `color`. A blocking read of stdin is a safe point: it ends with the
    /// task's exit once the root token is cancelled, so a cancelled run is not
    /// stuck behind a prompt.
    pub fn new(signals: Arc<Signals>, color: StreamColor) -> Self {
        let stdin = stream::Readable::from(std::io::stdin());
        stdin.set_stdin_interrupt(Arc::new(move || {
            signals
                .should_unwind()
                .then(|| std::io::Error::other(signals.exit_for()))
        }));
        Self {
            stdout: stream::Writable::stdout(std::io::stdout(), color.stdout),
            stderr: stream::Writable::stderr(std::io::stderr(), color.stderr),
            stdin,
        }
    }
}

#[starlark_value(type = "std.io.Stdio")]
impl<'v> values::StarlarkValue<'v> for Stdio {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("stdio_methods", stdio_methods);
        Some(RES.methods())
    }
}

starlark_simple_value!(Stdio);

#[starlark_module]
pub(crate) fn stdio_methods(registry: &mut MethodsBuilder) {
    /// Returns a writable stream for the standard output of the current process.
    #[starlark(attribute)]
    fn stdout<'v>(this: values::Value) -> anyhow::Result<stream::Writable> {
        let this = this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(this.stdout.dupe())
    }

    /// Returns a writable stream for the standard error of the current process.
    #[starlark(attribute)]
    fn stderr<'v>(this: values::Value) -> anyhow::Result<stream::Writable> {
        let this = this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(this.stderr.dupe())
    }

    /// Returns a readable stream for the standard input of the current process.
    #[starlark(attribute)]
    fn stdin<'v>(this: values::Value) -> anyhow::Result<stream::Readable> {
        let this = this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(this.stdin.dupe())
    }

    /// Whether detail lines print: the `INFO:` asides a reader can do
    /// without. The `Console` feature decides for the running task: its preset
    /// (shown under `full` and `emoji`, hidden under `compact`, `agent` and
    /// `silent`) unless a verbose level overrides it: the `verbose` arg on the
    /// command line, `ASPECT_DEBUG` / `ASPECT_VERBOSE`, the arg in a
    /// `config.axl`. True before the console resolves unless the environment
    /// says `false`. `detail()` in `lib/environment.axl` follows it; `info()`
    /// always prints. True at the `debug` level too; see `debug`.
    #[starlark(attribute)]
    fn verbose<'v>(this: values::Value) -> anyhow::Result<bool> {
        this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(crate::diag::verbose())
    }

    /// Whether debug lines print: the verbose level is `debug`
    /// (`--console:verbose=debug`, `ASPECT_VERBOSE=debug` or `ASPECT_DEBUG=1`).
    /// `debug()` in `lib/environment.axl` follows it.
    #[starlark(attribute)]
    fn debug<'v>(this: values::Value) -> anyhow::Result<bool> {
        this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(crate::diag::debugging())
    }

    /// `text` with its URLs and the paths to files that exist linked as the
    /// CLI's own lines on stderr are (`print()`'s): an OSC 8 link where the
    /// terminal takes one, its URL written beside it on the macOS Terminal,
    /// none off a terminal or with links off. A link `text` already has is
    /// kept, and gets its URL beside it there too.
    fn linked<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] text: &str,
    ) -> anyhow::Result<String> {
        this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(text
            .split('\n')
            .map(crate::out::linked)
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// The editor a link to a file opens in, as the console makes them
    /// (`--console:file-hyperlink-format`, or `auto`'s pick): `(format,
    /// name)`, `format` the alias (`cursor`), `template` for a URL template
    /// of the user's, `none` when files are not linked; `name` its name for
    /// a person (`Cursor`), "" for none.
    fn editor<'v>(this: values::Value<'v>) -> anyhow::Result<(String, String)> {
        this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(editor_of(&crate::out::link_format()))
    }

    /// The URL that opens `path` at `line` in `format`'s editor (an alias
    /// `editors()` lists, or "" for the one `editor()` names), or None when
    /// that format links no files.
    fn editor_url<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] path: &str,
        #[starlark(require = named, default = 1)] line: u32,
        #[starlark(require = named, default = "")] format: &str,
    ) -> anyhow::Result<values::none::NoneOr<String>> {
        this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        let format = if format.is_empty() {
            crate::out::link_format()
        } else {
            bazel::console::links::LinkFormat::parse(format).map_err(anyhow::Error::msg)?
        };
        let url = bazel::console::links::FileLinks::new(format)
            .url(std::path::Path::new(path), (Some(line), None))
            .filter(|url| !url.starts_with("file:"));
        Ok(values::none::NoneOr::from_option(url))
    }

    /// The file and line a link to a file names, `(path, line)`, read back
    /// from its URL by the console's format (`editor()`) or any editor's
    /// alias, or None for any other URL (a web page's, a `file://` one). For
    /// a screen that shows the file itself.
    fn editor_target<'v>(
        this: values::Value<'v>,
        #[starlark(require = pos)] url: &str,
    ) -> anyhow::Result<values::none::NoneOr<(String, u32)>> {
        this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(values::none::NoneOr::from_option(target_of(
            url,
            &crate::out::link_format(),
        )))
    }

    /// The editors installed here a link to a file can open in, as
    /// `(format, name)`, in the order `auto` would pick them.
    fn editors<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<(String, String)>> {
        this.downcast_ref_err::<Stdio>().into_anyhow_result()?;
        Ok(bazel::console::links::LinkFormat::EDITORS
            .iter()
            .filter(|(command, _)| bazel::console::links::installed(command))
            .map(|(_, alias)| (alias.to_string(), editor_name(alias).to_string()))
            .collect())
    }
}

/// [`editor_target`](stdio_methods): `url` matched against `format`'s
/// template, then each alias's.
fn target_of(url: &str, format: &bazel::console::links::LinkFormat) -> Option<(String, u32)> {
    use bazel::console::links::LinkFormat;
    let current = match format {
        LinkFormat::Template(t) => Some(t.as_str()),
        LinkFormat::None => None,
    };
    current
        .into_iter()
        .chain(LinkFormat::ALIASES.iter().map(|(_, t)| *t))
        .find_map(|template| match_template(template, url))
}

/// `url` read by `template`'s `{path}`, `{line}`, `{column}` and `{host}`.
fn match_template(template: &str, url: &str) -> Option<(String, u32)> {
    let mut pattern = String::from("^");
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        pattern.push_str(&regex::escape(&rest[..i]));
        let Some(j) = rest[i..].find('}') else { break };
        pattern.push_str(match &rest[i + 1..i + j] {
            "path" => "(?P<path>/[^?#&]*?)",
            "line" => "(?P<line>[0-9]+)",
            "column" => "[0-9]+",
            "host" => "[^/]*",
            _ => ".*?",
        });
        rest = &rest[i + j + 1..];
    }
    pattern.push_str(&regex::escape(rest));
    pattern.push('$');
    let caps = regex::Regex::new(&pattern).ok()?.captures(url)?;
    let path = bazel::bes::percent_decode(caps.name("path")?.as_str());
    let line = caps
        .name("line")
        .and_then(|l| l.as_str().parse().ok())
        .unwrap_or(1);
    Some((path, line))
}

/// An editor's name for a person, by its format's alias.
fn editor_name(alias: &str) -> &'static str {
    match alias {
        "cursor" => "Cursor",
        "vscode" => "VS Code",
        "vscode-insiders" => "VS Code Insiders",
        "vscodium" => "VSCodium",
        "zed" => "Zed",
        "idea" => "IntelliJ IDEA",
        "macvim" => "MacVim",
        "textmate" => "TextMate",
        "file" => "the system's app for it",
        "kitty" => "kitty",
        _ => "",
    }
}

/// `(format, name)` of a link format: its alias, else `template`, or `none`.
fn editor_of(format: &bazel::console::links::LinkFormat) -> (String, String) {
    use bazel::console::links::LinkFormat;
    match format {
        LinkFormat::None => ("none".into(), String::new()),
        LinkFormat::Template(t) => match LinkFormat::ALIASES.iter().find(|(_, tmpl)| tmpl == t) {
            Some((alias, _)) => (alias.to_string(), editor_name(alias).to_string()),
            None => ("template".into(), "your editor".into()),
        },
    }
}

#[cfg(test)]
mod editor_tests {
    use super::editor_of;
    use bazel::console::links::LinkFormat;

    #[test]
    fn a_files_link_is_read_back_to_its_path_and_line() {
        use super::target_of;
        let cursor = LinkFormat::parse("cursor").unwrap();
        assert_eq!(
            target_of("cursor://file/w/src/a%20b.rs:12:4", &cursor),
            Some(("/w/src/a b.rs".into(), 12))
        );
        assert_eq!(
            target_of("idea://open?file=/w/x.java&line=7&column=1", &cursor),
            Some(("/w/x.java".into(), 7)),
            "another editor's alias"
        );
        assert_eq!(
            target_of("file://host/w/log.txt", &cursor),
            None,
            "no editor's: the console makes no file:// link"
        );
        assert_eq!(target_of("https://x.test/a", &cursor), None);
    }

    #[test]
    fn an_editor_is_named_by_its_format() {
        assert_eq!(
            editor_of(&LinkFormat::parse("cursor").unwrap()),
            ("cursor".into(), "Cursor".into())
        );
        assert_eq!(editor_of(&LinkFormat::None), ("none".into(), String::new()));
        assert_eq!(
            editor_of(&LinkFormat::parse("myed://open?f={path}").unwrap()).0,
            "template"
        );
    }
}
