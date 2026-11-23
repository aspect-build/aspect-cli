//! Clickable paths and URLs for a console shown on a terminal: OSC 8
//! hyperlinks, which take no columns, so a row of curses' progress area keeps
//! its width.
//!
//! [`Linker::link`] links each `http://` or `https://` URL to itself, and
//! each path to a file that exists as its [`LinkFormat`] says (a directory
//! only under a `file://` format, which an editor would open as a folder;
//! nothing under `none`): an absolute path as written, a relative one
//! (`src/a.cc:3:4`, `BUILD.bazel`) from the first of the linker's
//! directories it exists under. For Bazel's console those are the
//! workspace the build runs in, which Bazel's own paths are relative to,
//! then the directory the task runs from. A path `relativize_paths` made
//! relative it links itself, as only it knows the file (`BUILD.bazel` from
//! a package directory is that package's), even one that does not exist:
//! the linker leaves text already linked alone. A response file's path
//! (`@pkg/x.params`) is linked after its `@`. Only the path is linked, not
//! a `:line:col` after it. A path to nothing (a sandbox gone with its
//! action, a label, prose) or cut short (`src/a.c…`) is left alone. A URL
//! ends before trailing punctuation, a bracket that closes none of its own,
//! and a `…` that marks it cut.
//!
//! What a file's link opens is a [`LinkFormat`], as ripgrep's
//! `--hyperlink-format` is: an editor's URL scheme
//! (`vscode://file{path}:{line}:{column}`) at the line and column that
//! follow the path (`:12:3`, `(12,3)`, a Starlark traceback's
//! `", line 12, column 3`), `file://` (what the system opens a file with, at
//! no line), or none, which links URLs alone. [`LinkFormat::auto`] finds
//! the editor. Where a terminal shows no OSC 8 link but opens a URL written
//! out (Terminal.app), each link's URL is written beside it too
//! ([`FileLinks::text`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::bytes::Regex;

use super::ansi;
use super::forward::LineProcessor;

/// A URL, up to the first byte no URL holds unquoted; trailing punctuation
/// is trimmed after ([`url_end`]).
static URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s\x00-\x1f\x7f<>"'`]+"#).unwrap());

/// A run of the bytes a path is written with.
static PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z0-9._/~+@-]+").unwrap());

/// Whether a path written as [`PATH`] matches can go on through `b`, for
/// `relativize_paths`, which finds where a path ends and what it is joined
/// to as the linker will (`node_modules/@types/…`).
pub(crate) fn is_path_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"._/~+@-".contains(&b)
}

/// How long a path found to name nothing is taken to still name nothing:
/// long enough that a busy console does not look one up on every line,
/// short enough that a file made since is linked.
const MISS_FOR: std::time::Duration = std::time::Duration::from_secs(1);

/// How many paths a [`Linker`] remembers resolving before forgetting them
/// all.
const REMEMBERED: usize = 4096;

/// The longest record linked: a longer one (a command line) is left as it is.
const MAX_RECORD: usize = 8192;

/// What a link to a file opens: ripgrep's `--hyperlink-format`, an alias or
/// a template over `{path}`, `{line}`, `{column}` and `{host}`, or nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkFormat {
    /// Files are not linked; URLs still are.
    None,
    /// A URL template, `{path}` its file's absolute path, percent-encoded,
    /// `{line}` and `{column}` where in it (1 when not said).
    Template(String),
}

impl LinkFormat {
    /// The names [`LinkFormat::parse`] takes, ripgrep's, each with the
    /// template it stands for.
    pub const ALIASES: &[(&str, &str)] = &[
        ("file", "file://{host}{path}"),
        ("vscode", "vscode://file{path}:{line}:{column}"),
        (
            "vscode-insiders",
            "vscode-insiders://file{path}:{line}:{column}",
        ),
        ("cursor", "cursor://file{path}:{line}:{column}"),
        ("vscodium", "vscodium://file{path}:{line}:{column}"),
        ("zed", "zed://file{path}:{line}:{column}"),
        (
            "idea",
            "idea://open?file={path}&line={line}&column={column}",
        ),
        (
            "macvim",
            "mvim://open?url=file://{path}&line={line}&column={column}",
        ),
        (
            "textmate",
            "txmt://open?url=file://{path}&line={line}&column={column}",
        ),
        ("kitty", "file://{host}{path}#{line}"),
    ];

    /// `none`, an alias, or a template naming `{path}`.
    pub fn parse(value: &str) -> Result<LinkFormat, String> {
        if value == "none" {
            return Ok(LinkFormat::None);
        }
        if let Some((_, template)) = Self::ALIASES.iter().find(|(name, _)| *name == value) {
            return Ok(LinkFormat::Template(template.to_string()));
        }
        if value.contains("{path}") && value.contains("://") {
            return Ok(LinkFormat::Template(value.to_string()));
        }
        let names: Vec<&str> = std::iter::once("none")
            .chain(Self::ALIASES.iter().map(|(name, _)| *name))
            .collect();
        Err(format!(
            "{value:?} is not a hyperlink format: expected auto, {}, or a URL template naming {{path}}",
            names.join(", ")
        ))
    }

    /// The editors `auto` finds a person's choice of, by the command that
    /// starts each, in the order picked when more than one is installed.
    pub const EDITORS: &[(&str, &str)] = &[
        ("cursor", "cursor"),
        ("code", "vscode"),
        ("code-insiders", "vscode-insiders"),
        ("zed", "zed"),
        ("codium", "vscodium"),
        ("idea", "idea"),
        ("mvim", "macvim"),
        ("mate", "textmate"),
    ];

    /// The format for a terminal that says nothing of its own, the first
    /// of:
    ///
    /// - on a remote machine (SSH, a dev container, Codespaces), none, as
    ///   a local editor cannot open its paths;
    /// - in an editor's integrated terminal, that editor: Cursor, VSCodium,
    ///   VS Code Insiders or VS Code, each setting `TERM_PROGRAM=vscode`
    ///   and told apart by the program its git extension names
    ///   (`VSCODE_GIT_ASKPASS_NODE`); none for any other, a fork not known,
    ///   a remote VS Code server or code-server;
    /// - the editor `$VISUAL`, else `$EDITOR`, starts (`code --wait`), when
    ///   it is one of [`LinkFormat::EDITORS`]; a terminal editor (`vim`)
    ///   cannot be opened by a click;
    /// - the first of [`LinkFormat::EDITORS`] installed, `on_path`;
    /// - none: a `file://` link opens whatever the system picks, at no
    ///   line.
    pub fn auto(
        var: impl Fn(&str) -> Option<std::ffi::OsString>,
        on_path: impl Fn(&str) -> bool,
    ) -> LinkFormat {
        let has =
            |name: &str, want: &str| var(name).is_some_and(|v| v.to_string_lossy().contains(want));
        let alias = |name: &str| Self::parse(name).expect("an alias");
        if ["SSH_CONNECTION", "REMOTE_CONTAINERS", "CODESPACES"]
            .iter()
            .any(|v| var(v).is_some())
        {
            return LinkFormat::None;
        }
        // The program the editor's git extension gives the shell, its own:
        // the app's bundle on macOS (`Cursor.app/…/Cursor Helper (Plugin)`),
        // its executable on Linux (`/usr/share/codium/codium`).
        let exe = var("VSCODE_GIT_ASKPASS_NODE").map(|v| v.to_string_lossy().into_owned());
        let named = |app: &str, file: &str| {
            exe.as_deref().is_some_and(|p| {
                p.contains(&format!("/{app}.app/"))
                    || std::path::Path::new(p)
                        .file_name()
                        .is_some_and(|f| f == file)
            })
        };
        if has("TERM_PROGRAM", "Cursor")
            || var("CURSOR_TRACE_ID").is_some()
            || named("Cursor", "cursor")
        {
            return alias("cursor");
        }
        if has("TERM_PROGRAM", "vscode") {
            if named("VSCodium", "codium") {
                return alias("vscodium");
            }
            if named("Visual Studio Code - Insiders", "code-insiders")
                || has("TERM_PROGRAM_VERSION", "insider")
            {
                return alias("vscode-insiders");
            }
            // VS Code itself, or one whose git extension names nothing; a
            // fork, a remote server's `node` or code-server's names
            // another program, whose paths a local VS Code may not open.
            return if exe.is_none() || named("Visual Studio Code", "code") {
                alias("vscode")
            } else {
                LinkFormat::None
            };
        }
        let chosen = ["VISUAL", "EDITOR"].iter().find_map(|name| {
            let command = editor_command(&var(name)?.to_string_lossy())?;
            Self::EDITORS
                .iter()
                .find(|(c, _)| *c == command)
                .map(|(_, a)| *a)
        });
        if let Some(name) = chosen {
            return alias(name);
        }
        Self::EDITORS
            .iter()
            .find(|(command, _)| on_path(command))
            .map_or(LinkFormat::None, |(_, name)| alias(name))
    }
}

/// The file name of the program a `$VISUAL`/`$EDITOR` value starts, read
/// as a shell would: quotes taken off (`"/Applications/Visual Studio
/// Code.app/…/bin/code" --wait`), and `env` and `NAME=value` before it
/// passed over (`env FOO=1 code`).
fn editor_command(value: &str) -> Option<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut quoted = false;
    for c in value.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                quoted = true;
            }
            None if c.is_whitespace() => {
                if quoted || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                quoted = false;
            }
            None => word.push(c),
        }
    }
    if quoted || !word.is_empty() {
        words.push(word);
    }
    let command = words
        .into_iter()
        .find(|w| w != "env" && !(w.contains('=') && !w.starts_with('/')))?;
    Some(command.rsplit('/').next().unwrap_or(&command).to_string())
}

/// Whether `command` is an executable file in a directory of `PATH`.
pub fn on_path(command: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            std::fs::metadata(dir.join(command))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    })
}

/// How links to files are made: on what host, in what [`LinkFormat`], and
/// whether each link's URL is also written beside it (`text`), for a
/// terminal that shows no OSC 8 link but opens a URL written out
/// (Terminal.app, by Cmd+double-click).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLinks {
    pub host: String,
    pub format: LinkFormat,
    pub text: bool,
}

impl FileLinks {
    /// `file://` links on this host, for a caller that does not choose.
    pub fn file() -> FileLinks {
        FileLinks::new(LinkFormat::parse("file").expect("an alias"))
    }

    pub fn new(format: LinkFormat) -> FileLinks {
        FileLinks {
            host: hostname(),
            format,
            text: false,
        }
    }

    /// These links with each URL written beside its link too, when `text`.
    pub fn written(mut self, text: bool) -> FileLinks {
        self.text = text;
        self
    }

    /// A link to `url` showing `shown`, then `said` (the `:12:3` after it),
    /// then, when written out, ` <url>`.
    pub fn link(&self, url: &str, shown: &[u8], said: &[u8]) -> Vec<u8> {
        let mut out = osc8(url, shown);
        out.extend_from_slice(said);
        if self.text {
            out.extend_from_slice(format!(" <{url}>").as_bytes());
        }
        out
    }

    /// The URL a link to `path` opens, at `at` (line, column) when the text
    /// says, or None when files are not linked, or for a directory in an
    /// editor, which would open it as a folder.
    pub fn url(&self, path: &Path, at: (Option<u32>, Option<u32>)) -> Option<String> {
        let LinkFormat::Template(template) = &self.format else {
            return None;
        };
        if !template.starts_with("file://") && path.is_dir() {
            return None;
        }
        Some(
            template
                .replace("{host}", &self.host)
                .replace("{path}", &encoded_path(path))
                .replace("{line}", &at.0.unwrap_or(1).to_string())
                .replace("{column}", &at.1.unwrap_or(1).to_string()),
        )
    }
}

/// Where in a file the text after its path, escapes removed, points:
/// `:12:3`, `:12`, `(12,3)`, `(12, 3)`, `(12)`, or a Starlark traceback's
/// `", line 12, column 3`, a number not running on into a word (`:12abc`).
pub fn position(after: &[u8]) -> (Option<u32>, Option<u32>) {
    located(after).map_or((None, None), |(line, column, _)| (line, column))
}

/// [`position`], and how many bytes of `after` say it.
fn located(after: &[u8]) -> Option<(Option<u32>, Option<u32>, usize)> {
    static AT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(concat!(
            r"^(?::(\d+)(?::(\d+))?|\((\d+)(?:, ?(\d+))?\)",
            r#"|", line (\d+)(?:, column (\d+))?)"#,
        ))
        .unwrap()
    });
    let c = AT.captures(after)?;
    let end = c.get(0).unwrap().end();
    if after.get(end).is_some_and(u8::is_ascii_alphanumeric) {
        return None;
    }
    let num = |i: usize| {
        c.get(i)
            .and_then(|m| std::str::from_utf8(m.as_bytes()).ok()?.parse().ok())
    };
    Some((
        num(1).or(num(3)).or(num(5)),
        num(2).or(num(4)).or(num(6)),
        end,
    ))
}

/// Links what a console record names; see the module docs.
pub struct Linker {
    /// Where a relative path may be from, in the order tried.
    bases: Vec<PathBuf>,
    /// How files are linked.
    pub(crate) files: FileLinks,
    /// What each path looked up names, by the path as written: the file, or
    /// None for nothing as of when it was looked up.
    resolved: HashMap<Vec<u8>, (Option<PathBuf>, std::time::Instant)>,
}

impl Linker {
    /// A linker resolving a relative path from the first of `bases` it
    /// exists under.
    pub fn new(bases: Vec<PathBuf>) -> Self {
        Self {
            bases,
            files: FileLinks::file(),
            resolved: HashMap::new(),
        }
    }

    /// This linker making its files' links as `files` says.
    pub fn with_files(mut self, files: FileLinks) -> Self {
        self.files = files;
        self
    }

    /// Make files' links in `format` from now on.
    pub fn set_format(&mut self, format: LinkFormat) {
        self.files.format = format;
    }

    /// `record` with each URL and each path to something that exists
    /// linked; escape sequences, and text already linked, kept as they are.
    pub fn link(&mut self, record: &[u8]) -> Vec<u8> {
        self.link_with(record, &[])
    }

    /// [`Linker::link`], with `known` links put back where they were: each
    /// at its byte offset among `record`'s text (its escapes not counted),
    /// whatever its text would otherwise link to.
    pub fn link_with(&mut self, record: &[u8], known: &[Known]) -> Vec<u8> {
        if record.len() > MAX_RECORD
            || known.is_empty() && !contains(record, b"/") && !contains(record, b".")
        {
            return record.to_vec();
        }
        // The record's text, escapes removed, where a path's line is read
        // though colour sets it apart (`src/a.ts` `:` `12`).
        let plain = ansi::strip(record);
        let mut out = Vec::with_capacity(record.len());
        let mut inside = false;
        let mut at = 0;
        for (escape, seg) in ansi::segments(record) {
            if escape {
                if let Some(url) = osc8_url(seg) {
                    inside = !url.is_empty();
                }
                out.extend_from_slice(seg);
                continue;
            }
            if inside {
                out.extend_from_slice(seg);
            } else {
                out.extend(self.text(seg, at, known, &plain));
            }
            at += seg.len();
        }
        out
    }

    /// `text`, at byte `base` among its record's text, linked.
    fn text(&mut self, text: &[u8], base: usize, known: &[Known], plain: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len());
        let mut at = 0;
        let mut urls = URL.find_iter(text).peekable();
        while at < text.len() {
            let next_url = urls.peek().map(|m| m.start());
            let stop = next_url.unwrap_or(text.len());
            self.paths(&text[at..stop], base + at, known, plain, &mut out);
            at = stop;
            if let Some(m) = urls.next() {
                let end = m.start() + url_end(m.as_bytes());
                out.extend(osc8(
                    &url_target(&text[m.start()..end]),
                    &text[m.start()..end],
                ));
                // What was trimmed from the URL's end is text again.
                self.paths(&text[end..m.end()], base + end, known, plain, &mut out);
                at = m.end();
            }
        }
        out
    }

    /// `text`, at byte `base` among its record's text and holding no URL,
    /// onto `out` with its paths linked.
    fn paths(
        &mut self,
        text: &[u8],
        base: usize,
        known: &[Known],
        plain: &[u8],
        out: &mut Vec<u8>,
    ) {
        let mut at = 0;
        for m in PATH.find_iter(text) {
            if m.start() < at {
                continue;
            }
            // A link put back here, or after a response file's `@`.
            let put = [m.start(), m.start() + 1].into_iter().find_map(|start| {
                let k = known.iter().find(|k| k.at == base + start)?;
                text[start..].starts_with(&k.shown).then_some((start, k))
            });
            if let Some((start, k)) = put {
                out.extend_from_slice(&text[at..start]);
                out.extend(osc8(&k.url, &k.shown));
                at = start + k.shown.len();
                continue;
            }
            let (mut start, mut end) = (m.start(), m.end());
            // A sentence's full stop, or `..` after a path, is not part of it.
            while end > start && text[end - 1] == b'.' {
                end -= 1;
            }
            // After a `:` it is a label's target (`//pkg:BUILD`) or a URL's
            // rest, not a path of its own.
            if start > 0 && text[start - 1] == b':' {
                continue;
            }
            // Cut short, it names some other file, if any.
            if text[end..].starts_with(ELLIPSIS) || end == start {
                continue;
            }
            let mut file = self.resolve(&text[start..end]);
            // A response file (`@pkg/x.params`), not a scoped package's
            // directory (`@types/x`) or a label (`@repo//pkg`).
            if file.is_none()
                && text[start] == b'@'
                && !text[start..end].windows(2).any(|w| w == b"//")
            {
                file = self.resolve(&text[start + 1..end]);
                if file.is_some() {
                    start += 1;
                }
            }
            let after = plain.get(base + end..).unwrap_or_default();
            let Some(url) = file.and_then(|f| self.files.url(&f, position(after))) else {
                continue;
            };
            out.extend_from_slice(&text[at..start]);
            out.extend(osc8(&url, &text[start..end]));
            at = end;
        }
        out.extend_from_slice(&text[at..]);
    }

    /// The file `path` names, when it could name one and does: it has a `/`
    /// or a `.` past its first byte and a letter, is not a label (`//pkg`),
    /// and exists, as written when absolute, else under one of the bases.
    fn resolve(&mut self, path: &[u8]) -> Option<PathBuf> {
        let shaped = path.len() > 1
            && path[1..].iter().any(|&b| b == b'/' || b == b'.')
            && path.iter().any(u8::is_ascii_alphabetic)
            && !path.starts_with(b"//");
        if !shaped {
            return None;
        }
        if let Some((known, at)) = self.resolved.get(path)
            && (known.is_some() || at.elapsed() < MISS_FOR)
        {
            return known.clone();
        }
        if self.resolved.len() >= REMEMBERED {
            self.resolved.clear();
        }
        use std::os::unix::ffi::OsStrExt;
        let written = Path::new(std::ffi::OsStr::from_bytes(path));
        let found = if written.is_absolute() {
            written.exists().then(|| written.to_path_buf())
        } else {
            self.bases
                .iter()
                .map(|b| b.join(written))
                .find(|p| p.exists())
        };
        self.resolved
            .insert(path.to_vec(), (found.clone(), std::time::Instant::now()));
        found
    }
}

/// The stage that links what Bazel's records name, last before the
/// reports, for a terminal.
pub struct Links {
    pub linker: Linker,
    /// See [`Links::under_pty`].
    pty: bool,
    /// Whether the record going by is a row curses draws.
    drawn: bool,
}

impl Links {
    pub fn new(linker: Linker) -> Self {
        Links {
            linker,
            pty: false,
            drawn: false,
        }
    }

    /// For a PTY capture: a row of Bazel's progress area gets no URL
    /// written beside its links, which would widen a row curses redraws.
    pub fn under_pty(mut self, pty: bool) -> Self {
        self.pty = pty;
        self
    }
}

impl LineProcessor for Links {
    fn see(&mut self, line: &crate::console::line::Line) {
        self.drawn = line.drawn;
    }

    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        let linked = self.linker.link(record);
        if self.linker.files.text && !(self.pty && self.drawn) {
            return Some(written_urls(&linked));
        }
        Some(linked)
    }
}

/// `record` with each link to a file's URL written beside it too, after
/// the path and the position said after it, as the text reads with its
/// escapes removed (`src/a.ts` `:9:4` <url>): for a terminal that shows no
/// OSC 8 link but opens a URL written out. A URL's own link
/// (`https://…`) shows it already.
pub fn written_urls(record: &[u8]) -> Vec<u8> {
    let plain = ansi::strip(record);
    // Where in `record` each byte of `plain` starts.
    let mut starts = Vec::with_capacity(plain.len() + 1);
    let mut ends = Vec::new();
    let mut open: Option<String> = None;
    let mut at = 0;
    for (escape, seg) in ansi::segments(record) {
        if escape {
            if let Some(url) = osc8_url(seg) {
                ends.extend(open.take().map(|url| (starts.len(), url)));
                if !url.is_empty() {
                    open = Some(String::from_utf8_lossy(url).into_owned());
                }
            }
        } else {
            starts.extend(at..at + seg.len());
        }
        at += seg.len();
    }
    ends.extend(open.map(|url| (starts.len(), url)));
    let mut inserts: Vec<(usize, String)> = ends
        .into_iter()
        .filter(|(_, url)| !url.starts_with("http://") && !url.starts_with("https://"))
        .map(|(end, url)| {
            let said = located(&plain[end..]).map_or(0, |(.., len)| len);
            let at = starts.get(end + said).copied().unwrap_or(record.len());
            (at, url)
        })
        .collect();
    inserts.sort_by_key(|(at, _)| *at);
    let mut out = Vec::with_capacity(record.len() + inserts.len() * 64);
    let mut from = 0;
    for (at, url) in inserts {
        out.extend_from_slice(&record[from..at]);
        out.extend_from_slice(format!(" <{url}>").as_bytes());
        from = at;
    }
    out.extend_from_slice(&record[from..]);
    out
}

/// The URL an OSC 8 sequence links to, empty for the one that ends a link,
/// or None for another escape.
fn osc8_url(seq: &[u8]) -> Option<&[u8]> {
    let rest = seq.strip_prefix(b"\x1b]8;")?;
    let uri = &rest[rest.iter().position(|&b| b == b';')? + 1..];
    let end = uri
        .iter()
        .position(|&b| b == 0x07 || b == 0x1b)
        .unwrap_or(uri.len());
    Some(&uri[..end])
}

/// A link in a record: its URL and the text it shows, at a byte offset
/// among the record's text (escapes not counted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    pub at: usize,
    pub shown: Vec<u8>,
    pub url: String,
}

/// Each link in `record`, for a stage that quotes its text without its
/// escapes and links it again where it was.
pub fn links_in(record: &[u8]) -> Vec<Known> {
    let mut found = Vec::new();
    let mut open: Option<Known> = None;
    let mut at = 0;
    for (escape, seg) in ansi::segments(record) {
        if escape {
            if let Some(url) = osc8_url(seg) {
                found.extend(open.take().filter(|k| !k.shown.is_empty()));
                if !url.is_empty() {
                    open = Some(Known {
                        at,
                        shown: Vec::new(),
                        url: String::from_utf8_lossy(url).into_owned(),
                    });
                }
            }
            continue;
        }
        if let Some(k) = &mut open {
            k.shown.extend_from_slice(seg);
        }
        at += seg.len();
    }
    found.extend(open.filter(|k| !k.shown.is_empty()));
    found
}

/// Where a URL found by [`URL`] ends: at a `…` that marks it cut, then
/// before trailing punctuation and before a `)`, `]` or `}` that closes none
/// of its own.
fn url_end(url: &[u8]) -> usize {
    let mut end = url
        .windows(ELLIPSIS.len())
        .position(|w| w == ELLIPSIS)
        .unwrap_or(url.len());
    loop {
        match url[..end].last() {
            Some(b'.' | b',' | b';' | b':' | b'!' | b'?') => end -= 1,
            Some(&close @ (b')' | b']' | b'}')) => {
                let open = match close {
                    b')' => b'(',
                    b']' => b'[',
                    _ => b'{',
                };
                let opened = url[..end].iter().filter(|&&b| b == open).count();
                let closed = url[..end].iter().filter(|&&b| b == close).count();
                if closed > opened {
                    end -= 1;
                } else {
                    return end;
                }
            }
            _ => return end,
        }
    }
}

/// `…`, which a quoted line ends with when it was cut.
const ELLIPSIS: &[u8] = "…".as_bytes();

/// A URL as an OSC 8 target, which takes printable ASCII only: any other
/// byte percent-encoded.
fn url_target(url: &[u8]) -> String {
    let mut out = String::with_capacity(url.len());
    for &b in url {
        if (0x21..0x7f).contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// `shown` as an OSC 8 hyperlink to `url`.
pub fn osc8(url: &str, shown: &[u8]) -> Vec<u8> {
    let mut out = format!("\x1b]8;;{url}\x1b\\").into_bytes();
    out.extend_from_slice(shown);
    out.extend_from_slice(b"\x1b]8;;\x1b\\");
    out
}

/// `path` as a `file://` URL on `host`, percent-encoded.
pub fn file_url(host: &str, path: &Path) -> String {
    format!("file://{host}{}", encoded_path(path))
}

/// `path`, percent-encoded for a URL.
fn encoded_path(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut url = String::new();
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

/// `path` as an OSC 8 hyperlink to it as `files` says, its text the path
/// itself; just the path when files are not linked.
pub fn file_link(files: &FileLinks, path: &Path) -> String {
    let shown = path.display().to_string();
    match files.url(path, (None, None)) {
        Some(url) => String::from_utf8(files.link(&url, shown.as_bytes(), b"")).unwrap_or(shown),
        None => shown,
    }
}

pub(crate) fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is valid for its length; gethostname NUL-terminates what
    // fits, and a failure leaves the name empty, a link to the local host.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    let end = buf.iter().position(|&c| c == 0).unwrap_or(0);
    if ok {
        String::from_utf8_lossy(&buf[..end]).into_owned()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linker() -> (tempfile::TempDir, Linker) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.cc"), "").unwrap();
        std::fs::write(dir.path().join("BUILD.bazel"), "").unwrap();
        let mut l = Linker::new(vec![dir.path().to_path_buf()]);
        l.files.host = String::new();
        (dir, l)
    }

    fn link(l: &mut Linker, s: &str) -> String {
        String::from_utf8(l.link(s.as_bytes())).unwrap()
    }

    fn file(dir: &Path, rel: &str, shown: &str) -> String {
        format!(
            "\x1b]8;;file://{}/{rel}\x1b\\{shown}\x1b]8;;\x1b\\",
            dir.display()
        )
    }

    #[test]
    fn a_path_that_exists_links_to_its_file_without_its_line() {
        let (dir, mut l) = linker();
        assert_eq!(
            link(&mut l, "ERROR: src/a.cc:3:4: expected ';'"),
            format!(
                "ERROR: {}:3:4: expected ';'",
                file(dir.path(), "src/a.cc", "src/a.cc")
            )
        );
        assert_eq!(
            link(&mut l, "DEBUG: BUILD.bazel:6:6: noisy."),
            format!(
                "DEBUG: {}:6:6: noisy.",
                file(dir.path(), "BUILD.bazel", "BUILD.bazel")
            )
        );
        let abs = format!("{}/src/a.cc", dir.path().display());
        assert_eq!(
            link(&mut l, &format!("see {abs}.")),
            format!("see {}.", file(dir.path(), "src/a.cc", &abs))
        );
    }

    #[test]
    fn what_names_nothing_is_left_alone() {
        let (_dir, mut l) = linker();
        for s in [
            "ERROR: src/gone.cc:1: missing",
            "//src:a.cc and @repo//pkg:t and //:BUILD.bazel",
            "version 1.2.3, e.g. this",
            "/tmp/_bazel_u/h/sandbox/linux-sandbox/7/execroot/_main/src/a.cc",
            "no paths here",
        ] {
            assert_eq!(link(&mut l, s), s);
        }
    }

    #[test]
    fn a_url_links_to_itself_without_trailing_punctuation() {
        let (_dir, mut l) = linker();
        let u = |s: &str| format!("\x1b]8;;{s}\x1b\\{s}\x1b]8;;\x1b\\");
        assert_eq!(
            link(
                &mut l,
                "INFO: Streaming build results to: https://app.aspect.build/inv/1a2b"
            ),
            format!(
                "INFO: Streaming build results to: {}",
                u("https://app.aspect.build/inv/1a2b")
            )
        );
        assert_eq!(
            link(&mut l, "(see http://x.test/a_(b).) next"),
            format!("(see {}.) next", u("http://x.test/a_(b)"))
        );
        assert_eq!(
            link(&mut l, "at https://x.test/src/a.cc, then"),
            format!("at {}, then", u("https://x.test/src/a.cc"))
        );
    }

    #[test]
    fn colour_and_existing_links_are_kept() {
        let (dir, mut l) = linker();
        assert_eq!(
            link(&mut l, "\x1b[31mERROR: \x1b[0msrc/a.cc:1: bad"),
            format!(
                "\x1b[31mERROR: \x1b[0m{}:1: bad",
                file(dir.path(), "src/a.cc", "src/a.cc")
            )
        );
        let linked = "\x1b]8;;file:///x\x1b\\src/a.cc\x1b]8;;\x1b\\";
        assert_eq!(
            link(&mut l, &format!("{linked} https://x.test")),
            format!("{linked} \x1b]8;;https://x.test\x1b\\https://x.test\x1b]8;;\x1b\\"),
            "text already linked is left alone, the rest linked"
        );
    }

    #[test]
    fn a_link_takes_no_columns() {
        let (_dir, mut l) = linker();
        let row = "    Compiling src/a.cc; 3s local, see https://x.test";
        assert_eq!(
            ansi::width(&l.link(row.as_bytes())),
            ansi::width(row.as_bytes())
        );
    }

    #[test]
    fn a_url_ends_at_an_unmatched_bracket_or_a_cut() {
        let (_dir, mut l) = linker();
        let u = |s: &str| format!("\x1b]8;;{s}\x1b\\{s}\x1b]8;;\x1b\\");
        assert_eq!(
            link(
                &mut l,
                "Error downloading [https://a.test/x.tar.gz, https://b.test/x.tar.gz] to y"
            ),
            format!(
                "Error downloading [{}, {}] to y",
                u("https://a.test/x.tar.gz"),
                u("https://b.test/x.tar.gz")
            )
        );
        assert_eq!(
            link(&mut l, "see https://a.test/w[1]"),
            format!("see {}", u("https://a.test/w[1]"))
        );
        assert_eq!(
            link(&mut l, "{https://a.test/x}"),
            format!("{{{}}}", u("https://a.test/x"))
        );
        assert_eq!(
            link(&mut l, "see https://a.test/inv/1a2…"),
            format!("see {}…", u("https://a.test/inv/1a2"))
        );
    }

    #[test]
    fn a_urls_target_is_printable_ascii() {
        let (_dir, mut l) = linker();
        assert_eq!(
            link(&mut l, "https://a.test/café"),
            "\x1b]8;;https://a.test/caf%C3%A9\x1b\\https://a.test/café\x1b]8;;\x1b\\"
        );
    }

    #[test]
    fn a_relative_path_resolves_from_the_first_base_it_is_under() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::create_dir_all(root.path().join("pkg/src")).unwrap();
        std::fs::write(root.path().join("src/a.ts"), "").unwrap();
        std::fs::write(root.path().join("pkg/src/a.ts"), "").unwrap();
        std::fs::write(root.path().join("pkg/only.ts"), "").unwrap();
        let mut l = Linker::new(vec![root.path().to_path_buf(), root.path().join("pkg")]);
        l.files.host = String::new();
        assert_eq!(
            link(&mut l, "src/a.ts:3"),
            format!("{}:3", file(root.path(), "src/a.ts", "src/a.ts")),
            "the workspace's, as Bazel writes it"
        );
        assert_eq!(
            link(&mut l, "only.ts:1"),
            format!("{}:1", file(root.path(), "pkg/only.ts", "only.ts")),
            "else the directory paths are relative to"
        );
    }

    #[test]
    fn a_path_made_after_it_was_named_is_linked() {
        let (dir, mut l) = linker();
        assert_eq!(link(&mut l, "made.txt here"), "made.txt here");
        std::fs::write(dir.path().join("made.txt"), "").unwrap();
        l.resolved.get_mut(b"made.txt".as_slice()).unwrap().1 -= MISS_FOR;
        assert_eq!(
            link(&mut l, "made.txt here"),
            format!("{} here", file(dir.path(), "made.txt", "made.txt"))
        );
    }

    /// A workspace of `files` under a temp dir, and what Bazel's console
    /// passes through: `stages` of `relativize_paths` (root, cwd), each
    /// linking with the workspace `build` (else the temp dir) as the build's,
    /// then the link stage with `bases`.
    fn console(
        files: &[&str],
        stages: &[(&str, Option<&str>)],
        build: Option<&str>,
    ) -> (tempfile::TempDir, impl FnMut(&str) -> String) {
        use crate::console::filters::{LinkTo, RelativizePaths};
        use crate::console::forward::LineProcessor as _;
        let dir = tempfile::tempdir().unwrap();
        for f in files {
            let at = dir.path().join(f);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, "").unwrap();
        }
        let top = dir.path().display().to_string();
        let ws = build.map_or(dir.path().to_path_buf(), |b| dir.path().join(b));
        let mut rels: Vec<RelativizePaths> = stages
            .iter()
            .map(|(root, cwd)| {
                RelativizePaths::new(
                    Some(root.replace("$T", &top)),
                    cwd.map(|c| c.replace("$T", &top)),
                )
                .linking(Some(LinkTo {
                    files: FileLinks {
                        host: String::new(),
                        format: LinkFormat::parse("file").unwrap(),
                        text: false,
                    },
                    workspace: ws.clone(),
                }))
            })
            .collect();
        let mut linker = Linker::new(vec![ws.clone(), dir.path().to_path_buf()]);
        linker.files.host = String::new();
        let mut links = Links::new(linker);
        let run = move |line: &str| {
            let mut record = line.replace("$T", &top).into_bytes();
            for r in &mut rels {
                record = r.process(&record).unwrap();
            }
            String::from_utf8(links.process(&record).unwrap()).unwrap()
        };
        (dir, run)
    }

    #[test]
    fn a_path_made_relative_links_to_the_file_it_named() {
        let (dir, mut run) = console(
            &["BUILD.bazel", "pkg/BUILD.bazel", "src/a.ts"],
            &[("$T", Some("$T/pkg"))],
            None,
        );
        assert_eq!(
            run("ERROR: $T/pkg/BUILD.bazel:3:1: x; src/a.ts:1"),
            format!(
                "ERROR: {}:3:1: x; {}:1",
                file(dir.path(), "pkg/BUILD.bazel", "BUILD.bazel"),
                file(dir.path(), "src/a.ts", "src/a.ts")
            ),
            "the package's BUILD.bazel, though the workspace has one; a tool's path the workspace's"
        );
    }

    #[test]
    fn a_shortened_path_and_a_literal_one_shown_alike_each_link_their_own() {
        let (dir, mut run) = console(
            &["BUILD.bazel", "pkg/BUILD.bazel"],
            &[("$T", Some("$T/pkg"))],
            None,
        );
        assert_eq!(
            run("ERROR: BUILD.bazel:9: stale; see $T/pkg/BUILD.bazel:1:1"),
            format!(
                "ERROR: {}:9: stale; see {}:1:1",
                file(dir.path(), "BUILD.bazel", "BUILD.bazel"),
                file(dir.path(), "pkg/BUILD.bazel", "BUILD.bazel")
            )
        );
    }

    #[test]
    fn an_execroot_path_links_in_the_builds_workspace_and_dots_link_nothing() {
        let (dir, mut run) = console(
            &["src/a.ts", "other/src/a.ts", "other/src/b.ts"],
            &[("$T", None)],
            Some("other"),
        );
        let other = dir.path().join("other");
        assert_eq!(
            run(
                "/o/execroot/_main/src/b.ts:1 and /o/sandbox/x/3/execroot/_main/src/a.ts:2; $T/... retry ..."
            ),
            format!(
                "{}:1 and {}:2; ... retry ...",
                file(&other, "src/b.ts", "src/b.ts"),
                file(&other, "src/a.ts", "src/a.ts")
            )
        );
    }

    #[test]
    fn a_scoped_package_path_links_its_own_file() {
        let (dir, mut run) = console(
            &[
                "node_modules/@types/x/index.d.ts",
                "pkg/node_modules/@types/x/index.d.ts",
            ],
            &[("$T", Some("$T/pkg"))],
            None,
        );
        let out = run("ERROR: $T/pkg/node_modules/@types/x/index.d.ts:1:1: bad");
        assert_eq!(
            out,
            format!(
                "ERROR: {}:1:1: bad",
                file(
                    dir.path(),
                    "pkg/node_modules/%40types/x/index.d.ts",
                    "node_modules/@types/x/index.d.ts"
                )
            )
        );
    }

    #[test]
    fn stacked_stages_link_each_path_to_its_own_file_in_any_order() {
        let (dir, mut run) = console(
            &[
                "ws/BUILD.bazel",
                "ws/pkg/BUILD.bazel",
                "ws/pkg/@scope/foo/index.ts",
                "vend/BUILD.bazel",
            ],
            &[("$T/ws", Some("$T/ws/pkg")), ("$T/vend", None)],
            Some("ws"),
        );
        let pkg = file(dir.path(), "ws/pkg/BUILD.bazel", "BUILD.bazel");
        let vend = file(dir.path(), "vend/BUILD.bazel", "BUILD.bazel");
        assert_eq!(
            run("ERROR: $T/ws/pkg/BUILD.bazel:1:1: x; see $T/vend/BUILD.bazel:2"),
            format!("ERROR: {pkg}:1:1: x; see {vend}:2")
        );
        assert_eq!(
            run("ERROR: $T/vend/BUILD.bazel:2: x; see $T/ws/pkg/BUILD.bazel:1:1"),
            format!("ERROR: {vend}:2: x; see {pkg}:1:1")
        );
        assert_eq!(
            run("at $T/ws/pkg/@scope/foo/index.ts:2"),
            format!(
                "at {}:2",
                file(
                    dir.path(),
                    "ws/pkg/%40scope/foo/index.ts",
                    "@scope/foo/index.ts"
                )
            ),
            "not `.@scope`"
        );
    }

    #[test]
    fn a_response_files_path_links_after_its_at() {
        let (dir, mut run) = console(&["MODULE.bazel", "pkg/x.params"], &[("$T", None)], None);
        assert_eq!(
            run("argfile @$T/pkg/x.params and @MODULE.bazel, not @repo//pkg"),
            format!(
                "argfile @{} and @{}, not @repo//pkg",
                file(dir.path(), "pkg/x.params", "pkg/x.params"),
                file(dir.path(), "MODULE.bazel", "MODULE.bazel")
            )
        );
    }

    #[test]
    fn a_quoted_lines_links_carry_over() {
        let (dir, mut run) = console(
            &["BUILD.bazel", "pkg/BUILD.bazel"],
            &[("$T", Some("$T/pkg"))],
            None,
        );
        let shown = run("ERROR: $T/pkg/BUILD.bazel:1: x");
        let known = links_in(shown.as_bytes());
        let mut l = Linker::new(vec![dir.path().to_path_buf()]);
        l.files.host = String::new();
        let quoted = String::from_utf8(l.link_with(b"ERROR: BUILD.bazel:1: x", &known)).unwrap();
        assert_eq!(
            quoted,
            format!(
                "ERROR: {}:1: x",
                file(dir.path(), "pkg/BUILD.bazel", "BUILD.bazel")
            )
        );
    }

    #[test]
    fn a_shortened_path_to_nothing_links_the_file_it_named_not_anothers() {
        let (dir, mut run) = console(&["MODULE.bazel"], &[("$T", Some("$T/pkg"))], None);
        assert_eq!(
            run("ERROR: $T/pkg/MODULE.bazel:1:1: no such file"),
            format!(
                "ERROR: {}:1:1: no such file",
                file(dir.path(), "pkg/MODULE.bazel", "MODULE.bazel")
            )
        );
    }

    #[test]
    fn a_known_link_goes_back_in_a_line_with_no_path_of_its_own() {
        let (_dir, mut l) = linker();
        let known = [Known {
            at: 7,
            shown: b"packages".to_vec(),
            url: "file:///ws/packages".into(),
        }];
        assert_eq!(
            String::from_utf8(l.link_with(b"ERROR: packages: bad", &known)).unwrap(),
            "ERROR: \x1b]8;;file:///ws/packages\x1b\\packages\x1b]8;;\x1b\\: bad"
        );
    }

    #[test]
    fn a_path_cut_short_is_left_alone() {
        let (_dir, mut l) = linker();
        assert_eq!(link(&mut l, "see src/a.c…"), "see src/a.c…");
        assert_eq!(link(&mut l, "see src…"), "see src…");
    }

    #[test]
    fn a_hyperlink_format_is_an_alias_a_template_or_none() {
        let url = |f: &str, at| {
            FileLinks {
                host: "h".into(),
                format: LinkFormat::parse(f).unwrap(),
                text: false,
            }
            .url(Path::new("/w/a b.ts"), at)
        };
        assert_eq!(
            url("vscode", (Some(3), Some(4))).as_deref(),
            Some("vscode://file/w/a%20b.ts:3:4")
        );
        assert_eq!(
            url("cursor", (Some(3), None)).as_deref(),
            Some("cursor://file/w/a%20b.ts:3:1")
        );
        assert_eq!(
            url("file", (Some(3), Some(4))).as_deref(),
            Some("file://h/w/a%20b.ts"),
            "no line in a file URL"
        );
        assert_eq!(
            url("idea://open?file={path}&line={line}", (Some(9), None)).as_deref(),
            Some("idea://open?file=/w/a%20b.ts&line=9")
        );
        assert_eq!(url("none", (None, None)), None);
        assert!(
            LinkFormat::parse("vscod")
                .unwrap_err()
                .contains("expected auto, none, file, vscode")
        );
    }

    #[test]
    fn auto_is_the_editor_whose_terminal_this_is() {
        let auto = |vars: &[(&str, &str)]| {
            let vars: Vec<(String, String)> = vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            LinkFormat::auto(
                |k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.into()),
                |_| false,
            )
        };
        assert_eq!(
            auto(&[("TERM_PROGRAM", "Apple_Terminal")]),
            LinkFormat::None
        );
        assert_eq!(
            auto(&[("TERM_PROGRAM", "vscode")]),
            LinkFormat::parse("vscode").unwrap()
        );
        assert_eq!(
            auto(&[
                ("TERM_PROGRAM", "vscode"),
                (
                    "VSCODE_GIT_ASKPASS_NODE",
                    "/Applications/Cursor.app/Contents/Frameworks/Cursor Helper (Plugin).app/x"
                )
            ]),
            LinkFormat::parse("cursor").unwrap()
        );
        assert_eq!(
            auto(&[
                ("TERM_PROGRAM", "vscode"),
                ("TERM_PROGRAM_VERSION", "1.96.0-insider")
            ]),
            LinkFormat::parse("vscode-insiders").unwrap()
        );
    }

    #[test]
    fn a_path_links_at_the_line_after_it() {
        assert_eq!(position(b":12:3: error"), (Some(12), Some(3)));
        assert_eq!(position(b":12: x"), (Some(12), None));
        assert_eq!(position(b"(12,3): error TS1"), (Some(12), Some(3)));
        assert_eq!(position(b" at 12"), (None, None));
        let (_dir, mut l) = linker();
        l.set_format(LinkFormat::parse("vscode").unwrap());
        let out = String::from_utf8(l.link(b"src/a.cc:3:4: bad")).unwrap();
        assert!(
            out.starts_with("\x1b]8;;vscode://file/")
                && out.contains("/src/a.cc:3:4\x1b\\src/a.cc\x1b]8;;"),
            "{out:?}"
        );
        l.set_format(LinkFormat::None);
        assert_eq!(
            String::from_utf8(l.link(b"src/a.cc:3:4 at https://x.test")).unwrap(),
            "src/a.cc:3:4 at \x1b]8;;https://x.test\x1b\\https://x.test\x1b]8;;\x1b\\",
            "URLs linked whatever it says"
        );
    }

    #[test]
    fn auto_names_vs_code_and_forks_it_knows_on_this_machine() {
        let auto = |vars: &[(&str, &str)]| {
            let vars: Vec<(String, String)> = vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            LinkFormat::auto(
                |k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.into()),
                |_| false,
            )
        };
        let code =
            "/Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper (Plugin).app/x";
        assert_eq!(
            auto(&[
                ("TERM_PROGRAM", "vscode"),
                ("VSCODE_GIT_ASKPASS_NODE", code)
            ]),
            LinkFormat::parse("vscode").unwrap()
        );
        assert_eq!(
            auto(&[
                ("TERM_PROGRAM", "vscode"),
                (
                    "VSCODE_GIT_ASKPASS_NODE",
                    "/Applications/VSCodium.app/Contents/Frameworks/x"
                )
            ]),
            LinkFormat::parse("vscodium").unwrap()
        );
        assert_eq!(
            auto(&[
                ("TERM_PROGRAM", "vscode"),
                ("VSCODE_GIT_ASKPASS_NODE", "/Applications/SomeFork.app/x")
            ]),
            LinkFormat::None,
            "a fork not known links no files"
        );
        assert_eq!(
            auto(&[
                ("TERM_PROGRAM", "vscode"),
                ("VSCODE_GIT_ASKPASS_NODE", code),
                ("SSH_CONNECTION", "1 2 3 4")
            ]),
            LinkFormat::None,
            "a remote machine's paths are not this one's"
        );
        assert!(
            LinkFormat::parse("{path}").is_err(),
            "a template names a scheme"
        );
    }

    #[test]
    fn a_line_is_read_through_colour_and_a_directory_is_no_editor_link() {
        assert_eq!(position(b"(12, 3): error"), (Some(12), Some(3)));
        assert_eq!(position(b":12abc"), (None, None));
        let (_dir, mut l) = linker();
        l.set_format(LinkFormat::parse("vscode").unwrap());
        let pretty = String::from_utf8(
            l.link(b"\x1b[96msrc/a.cc\x1b[0m:\x1b[93m12\x1b[0m:\x1b[93m3\x1b[0m - error"),
        )
        .unwrap();
        assert!(
            pretty.contains("/src/a.cc:12:3\x1b\\"),
            "a line in its own colour: {pretty:?}"
        );
        assert!(
            !String::from_utf8(l.link(b"see src/ there"))
                .unwrap()
                .contains("\x1b]8"),
            "no editor link to a directory"
        );
        l.set_format(LinkFormat::parse("file").unwrap());
        assert!(
            String::from_utf8(l.link(b"see src/ there"))
                .unwrap()
                .contains("\x1b]8"),
            "a file link to one"
        );
    }

    fn auto_in(vars: &[(&str, &str)], installed: &[&str]) -> LinkFormat {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        LinkFormat::auto(
            |k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.into()),
            |c| installed.contains(&c),
        )
    }

    #[test]
    fn auto_tells_editors_by_the_program_their_terminal_names() {
        let vscode = LinkFormat::parse("vscode").unwrap();
        assert_eq!(
            auto_in(&[("TERM_PROGRAM", "vscode")], &[]),
            vscode,
            "its git extension off"
        );
        assert_eq!(
            auto_in(
                &[
                    ("TERM_PROGRAM", "vscode"),
                    ("VSCODE_IPC_HOOK_CLI", "/var/folders/x/T/vscode-ipc-1.sock")
                ],
                &[]
            ),
            vscode,
            "an IPC socket names no app"
        );
        assert_eq!(
            auto_in(
                &[
                    ("TERM_PROGRAM", "vscode"),
                    ("VSCODE_GIT_ASKPASS_NODE", "/usr/share/code/code")
                ],
                &[]
            ),
            vscode
        );
        assert_eq!(
            auto_in(
                &[
                    ("TERM_PROGRAM", "vscode"),
                    ("VSCODE_GIT_ASKPASS_NODE", "/usr/share/codium/codium")
                ],
                &[]
            ),
            LinkFormat::parse("vscodium").unwrap()
        );
        for server in [
            "/usr/lib/code-server/lib/node",
            "/home/code/.vscode-server/bin/x/node",
        ] {
            assert_eq!(
                auto_in(
                    &[
                        ("TERM_PROGRAM", "vscode"),
                        ("VSCODE_GIT_ASKPASS_NODE", server)
                    ],
                    &["code"]
                ),
                LinkFormat::None,
                "{server}: the paths are a server's"
            );
        }
    }

    #[test]
    fn auto_outside_an_editor_takes_the_one_chosen_else_installed() {
        let plain = [("TERM_PROGRAM", "iTerm.app")];
        assert_eq!(auto_in(&plain, &[]), LinkFormat::None);
        assert_eq!(
            auto_in(&plain, &["code"]),
            LinkFormat::parse("vscode").unwrap()
        );
        assert_eq!(
            auto_in(&plain, &["code", "cursor"]),
            LinkFormat::parse("cursor").unwrap(),
            "cursor first"
        );
        assert_eq!(
            auto_in(
                &[
                    ("TERM_PROGRAM", "iTerm.app"),
                    ("EDITOR", "/usr/local/bin/code --wait")
                ],
                &["cursor"]
            ),
            LinkFormat::parse("vscode").unwrap(),
            "the editor chosen beats the one installed"
        );
        assert_eq!(
            auto_in(
                &[
                    ("TERM_PROGRAM", "iTerm.app"),
                    ("VISUAL", "zed"),
                    ("EDITOR", "code")
                ],
                &[]
            ),
            LinkFormat::parse("zed").unwrap()
        );
        assert_eq!(
            auto_in(
                &[("TERM_PROGRAM", "iTerm.app"), ("EDITOR", "vim")],
                &["code"]
            ),
            LinkFormat::parse("vscode").unwrap(),
            "a terminal editor no click can open"
        );
        assert_eq!(
            auto_in(&[("SSH_CONNECTION", "1 2 3 4")], &["code"]),
            LinkFormat::None
        );
    }

    #[test]
    fn a_traceback_line_is_placed() {
        assert_eq!(
            position(b"\", line 12, column 3, in f"),
            (Some(12), Some(3))
        );
        assert_eq!(position(b"\", line 7"), (Some(7), None));
    }

    /// The final stage in Terminal.app, `format` its files' links.
    fn written(format: &str) -> (tempfile::TempDir, Links) {
        let (dir, mut l) = linker();
        l.files.format = LinkFormat::parse(format).unwrap();
        l.files.text = true;
        (dir, Links::new(l).under_pty(true))
    }

    fn url_in(out: &str) -> String {
        out.split("\x1b]8;;")
            .nth(1)
            .unwrap()
            .split('\x1b')
            .next()
            .unwrap()
            .to_string()
    }

    #[test]
    fn a_written_url_goes_after_the_position_however_coloured() {
        use crate::console::forward::LineProcessor as _;
        let (_dir, mut links) = written("vscode");
        let out = String::from_utf8(links.process(b"ERROR: src/a.cc:3:4: bad").unwrap()).unwrap();
        assert!(
            out.ends_with(&format!(
                "src/a.cc\x1b]8;;\x1b\\:3:4 <{}>: bad",
                url_in(&out)
            )),
            "{out:?}"
        );
        let out =
            String::from_utf8(links.process(b"x src/a.cc\x1b[2m:\x1b[0m9:4 q").unwrap()).unwrap();
        assert!(
            out.ends_with(&format!("\x1b[2m:\x1b[0m9:4 <{}> q", url_in(&out))),
            "{out:?}"
        );
    }

    #[test]
    fn a_progress_row_gets_no_written_url_under_a_pty() {
        use crate::console::forward::LineProcessor as _;
        let (_dir, mut links) = written("vscode");
        let row = b"\x1b[32m[3 / 10]\x1b[0m Compiling src/a.cc; 2s darwin-sandbox";
        let line = crate::console::line::Line {
            kind: crate::console::line::Kind::Progress,
            text: String::new(),
            fresh: false,
            coloured: true,
            indented: false,
            opens_output: false,
            drawn: true,
        };
        links.see(&line);
        let out = links.process(row).unwrap();
        assert_eq!(
            ansi::width(&out),
            ansi::width(row),
            "{:?}",
            String::from_utf8_lossy(&out)
        );
    }

    #[test]
    fn a_written_url_is_not_linked_again() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/a.ts"), "").unwrap();
        let ws = root.path().display().to_string();
        use crate::console::filters::{LinkTo, RelativizePaths};
        use crate::console::forward::LineProcessor as _;
        let files = FileLinks {
            host: String::new(),
            format: LinkFormat::parse("idea").unwrap(),
            text: true,
        };
        let mut rel = RelativizePaths::new(Some(ws.clone()), None).linking(Some(LinkTo {
            files: files.clone(),
            workspace: root.path().to_path_buf(),
        }));
        let mut linker = Linker::new(vec![root.path().to_path_buf()]);
        linker.files = files;
        let mut links = Links::new(linker);
        let shown = rel
            .process(format!("ERROR: {ws}/src/a.ts:3:4: bad").as_bytes())
            .unwrap();
        let out = String::from_utf8(links.process(&shown).unwrap()).unwrap();
        assert_eq!(out.matches("\x1b]8;;idea:").count(), 1, "{out:?}");
        assert_eq!(out.matches(" <idea://").count(), 1, "{out:?}");
    }

    #[test]
    fn an_editor_is_read_as_a_shell_would() {
        assert_eq!(editor_command("code --wait").as_deref(), Some("code"));
        assert_eq!(
            editor_command(
                "\"/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code\" --wait"
            )
            .as_deref(),
            Some("code")
        );
        assert_eq!(editor_command("env FOO=1 zed").as_deref(), Some("zed"));
        assert_eq!(editor_command("  ").as_deref(), None);
    }

    #[test]
    fn a_file_url_is_percent_encoded() {
        assert_eq!(
            file_url("h", Path::new("/tmp/a b.log")),
            "file://h/tmp/a%20b.log"
        );
    }
}
