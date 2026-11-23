//! Clickable paths and URLs for a console shown on a terminal: OSC 8
//! hyperlinks, which take no columns, so a row of curses' progress area keeps
//! its width.
//!
//! [`Linker::link`] links each `http://` or `https://` URL to itself, and
//! each path to a file or directory that exists to it as a `file://` URL: an
//! absolute path as written, a relative one (`src/a.cc:3:4`, `BUILD.bazel`)
//! from the first of the linker's directories it exists under. For Bazel's
//! console those are the workspace the build runs in, which Bazel's own
//! paths are relative to, then the directory the task runs from. A path
//! `relativize_paths` made relative it links itself, as only it knows the
//! file (`BUILD.bazel` from a package directory is that package's), even
//! one that does not exist: the linker leaves text already linked alone. A response file's path
//! (`@pkg/x.params`) is linked after its `@`. Only the path is linked, not a
//! `:line:col` after it. A path to nothing (a sandbox gone with its action, a
//! label, prose) or cut short (`src/a.c…`) is left alone. A URL ends before
//! trailing punctuation, a bracket that closes none of its own, and a `…`
//! that marks it cut.
//!
//! What a file's link opens is a [`LinkFormat`], as ripgrep's
//! `--hyperlink-format` is: an editor's URL scheme (`vscode://file{path}:{line}:{column}`)
//! with the line and column that follow the path, `file://` (what the
//! system opens a file with, at no line), or none, which links URLs alone.

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
        if value.contains("{path}") {
            return Ok(LinkFormat::Template(value.to_string()));
        }
        let names: Vec<&str> = std::iter::once("none")
            .chain(Self::ALIASES.iter().map(|(name, _)| *name))
            .collect();
        Err(format!(
            "{value:?} is not a hyperlink format: expected auto, {}, or a template naming {{path}}",
            names.join(", ")
        ))
    }

    /// The format for a terminal that says nothing of its own: the editor's
    /// whose integrated terminal this is, VS Code's or a fork's, as each
    /// sets `TERM_PROGRAM=vscode` and names its own app in the paths it
    /// gives the shell; files unlinked anywhere else, since a `file://`
    /// link opens whatever the system picks, at no line.
    pub fn auto(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> LinkFormat {
        let is =
            |name: &str, want: &str| var(name).is_some_and(|v| v.to_string_lossy().contains(want));
        let cursor = is("TERM_PROGRAM", "Cursor")
            || var("CURSOR_TRACE_ID").is_some()
            || is("VSCODE_GIT_ASKPASS_NODE", "Cursor")
            || is("VSCODE_IPC_HOOK_CLI", "Cursor");
        if cursor {
            return Self::parse("cursor").expect("an alias");
        }
        if is("TERM_PROGRAM", "vscode") {
            let insiders =
                is("TERM_PROGRAM_VERSION", "insider") || is("VSCODE_GIT_ASKPASS_NODE", "Insiders");
            return Self::parse(if insiders {
                "vscode-insiders"
            } else {
                "vscode"
            })
            .expect("an alias");
        }
        LinkFormat::None
    }
}

/// How links to files are made: on what host, in what [`LinkFormat`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLinks {
    pub host: String,
    pub format: LinkFormat,
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
        }
    }

    /// The URL a link to `path` opens, at `at` (line, column) when the text
    /// says, or None when files are not linked.
    pub fn url(&self, path: &Path, at: (Option<u32>, Option<u32>)) -> Option<String> {
        let LinkFormat::Template(template) = &self.format else {
            return None;
        };
        Some(
            template
                .replace("{host}", &self.host)
                .replace("{path}", &encoded_path(path))
                .replace("{line}", &at.0.unwrap_or(1).to_string())
                .replace("{column}", &at.1.unwrap_or(1).to_string()),
        )
    }
}

/// Where in a file the text after its path points: `:12:3`, `:12`,
/// `(12,3)` or `(12)`.
pub fn position(after: &[u8]) -> (Option<u32>, Option<u32>) {
    static AT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?::(\d+)(?::(\d+))?|\((\d+)(?:,(\d+))?\))").unwrap());
    let Some(c) = AT.captures(after) else {
        return (None, None);
    };
    let num = |i: usize| {
        c.get(i)
            .and_then(|m| std::str::from_utf8(m.as_bytes()).ok()?.parse().ok())
    };
    (num(1).or(num(3)), num(2).or(num(4)))
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
                out.extend(self.text(seg, at, known));
            }
            at += seg.len();
        }
        out
    }

    /// `text`, at byte `base` among its record's text, linked.
    fn text(&mut self, text: &[u8], base: usize, known: &[Known]) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len());
        let mut at = 0;
        let mut urls = URL.find_iter(text).peekable();
        while at < text.len() {
            let next_url = urls.peek().map(|m| m.start());
            let stop = next_url.unwrap_or(text.len());
            self.paths(&text[at..stop], base + at, known, &mut out);
            at = stop;
            if let Some(m) = urls.next() {
                let end = m.start() + url_end(m.as_bytes());
                out.extend(osc8(
                    &url_target(&text[m.start()..end]),
                    &text[m.start()..end],
                ));
                // What was trimmed from the URL's end is text again.
                self.paths(&text[end..m.end()], base + end, known, &mut out);
                at = m.end();
            }
        }
        out
    }

    /// `text`, at byte `base` among its record's text and holding no URL,
    /// onto `out` with its paths linked.
    fn paths(&mut self, text: &[u8], base: usize, known: &[Known], out: &mut Vec<u8>) {
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
            let Some(url) = file.and_then(|f| self.files.url(&f, position(&text[end..]))) else {
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
pub struct Links(pub Linker);

impl LineProcessor for Links {
    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        Some(self.0.link(record))
    }
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
        Some(url) => String::from_utf8(osc8(&url, shown.as_bytes())).unwrap_or(shown),
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
                    },
                    workspace: ws.clone(),
                }))
            })
            .collect();
        let mut linker = Linker::new(vec![ws.clone(), dir.path().to_path_buf()]);
        linker.files.host = String::new();
        let mut links = Links(linker);
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
            LinkFormat::auto(|k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.into()))
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
    fn a_file_url_is_percent_encoded() {
        assert_eq!(
            file_url("h", Path::new("/tmp/a b.log")),
            "file://h/tmp/a%20b.log"
        );
    }
}
