//! The stages that change what is shown: [`DropLines`], [`Rewrite`],
//! [`RelativizePaths`] and [`CollapseSimilar`].
//!
//! # Messages and their bodies
//!
//! Bazel writes a message as a header line (`WARNING: …`) and, often, body
//! lines under it: a traceback, a repository's call stack. These stages
//! match and hide messages by their header, never a line of action or test
//! output, and a hidden header takes only the indented lines right under it
//! ([`Body`]), and the `Repository rule … defined at:` line of bazel's own
//! call stacks; another unindented one such as `Analysis failed` stays,
//! since nothing marks it as the message's. Hiding a progress line, which
//! only plain lines from `--curses=no` have, hides that line and the blank
//! line bazel writes after it: bazel writes that progress as one line, so an
//! indented line after it is someone else's. Headers that open an action's
//! output (`INFO: From …`, an action's `ERROR:`) are never hidden. Where a
//! message starts at all is [`super::line`]'s to say.
//!
//! # Where curses is shown (`under_pty`)
//!
//! Bazel's curses UI redraws its progress area by moving the cursor up over
//! the lines it last wrote there. A progress line that went missing would make
//! that redraw erase a line above it, so where curses is shown no stage hides
//! a blank line or any line of the progress area, nor makes one wider than
//! bazel cropped it to the terminal ([`keeps_its_rows`]): a `rewrite` cuts it
//! back, `relativize_paths` leaves it be. Messages and their bodies are
//! written above the progress area and are safe to hide. A hidden record
//! still forwards its escape sequences (see [`super::forward`]).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use regex::Regex;
use regex::bytes::Regex as BytesRegex;

use super::ansi;
use super::forward::LineProcessor;
use super::line::{self, Body, Given, Kind, Line};
use super::report::Stats;

/// What [`DropLines`] and [`CollapseSimilar`] share: the classification of
/// each record and the body of a hidden message.
struct Hider {
    pty: bool,
    given: Given,
    body: Body,
    /// A progress line was hidden just now: in a pipe, the blank line bazel
    /// writes after one goes with it.
    after_hidden_progress: bool,
    stats: Arc<Stats>,
}

impl Hider {
    fn new(pty: bool, stats: Arc<Stats>) -> Self {
        Self {
            pty,
            given: Given::default(),
            body: Body::default(),
            after_hidden_progress: false,
            stats,
        }
    }

    /// What `record` is, and its message text as it reaches this stage;
    /// `None` when it belongs to what was hidden last and is hidden with it,
    /// which is counted.
    fn see(&mut self, record: &[u8]) -> Option<(Line, String)> {
        let line = self.given.take(record);
        if self.body.swallows(&line) {
            self.stats.hid(1);
            return None;
        }
        if std::mem::take(&mut self.after_hidden_progress) && line.kind == Kind::Blank && !self.pty
        {
            return None;
        }
        Some((line, line::message_of(record)))
    }

    /// A record a stage before this one hid went by: it still ends the open
    /// body if it is not part of it (a new message, say), and is not counted.
    fn skipped(&mut self, line: &Line) {
        self.body.swallows(line);
        self.after_hidden_progress = false;
    }

    /// Whether a stage may match and hide `line` at all: a message's header,
    /// or in a pipe one of bazel's coloured progress lines. Not the header of
    /// an action's output, whose output would be left under the wrong one.
    /// Anything else is hidden only as part of what was hidden before it.
    fn target(&self, line: &Line) -> bool {
        (line.kind == Kind::Header && !line.opens_output)
            || (line.kind == Kind::Progress && line.coloured && !self.pty)
    }

    /// Note that `line` was hidden, so a message's body goes too. With a
    /// progress line goes only the blank line after it, in a pipe; nothing
    /// with the header of an action's output.
    fn hid(&mut self, line: &Line) {
        if line.kind.is_progress() {
            self.after_hidden_progress = true;
        } else if !line.opens_output {
            self.body.open();
        }
    }
}

/// Hides each message (and, in a pipe, progress line) matching one of
/// `patterns`, body included.
pub struct DropLines {
    patterns: Vec<Regex>,
    hider: Hider,
}

impl DropLines {
    pub fn new(patterns: Vec<Regex>, pty: bool, stats: Arc<Stats>) -> Self {
        Self {
            patterns,
            hider: Hider::new(pty, stats),
        }
    }
}

impl LineProcessor for DropLines {
    fn see(&mut self, line: &Line) {
        self.hider.given.see(line);
    }

    fn skipped(&mut self, line: &Line) {
        self.hider.skipped(line);
    }

    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        let (line, text) = self.hider.see(record)?;
        if self.hider.target(&line) && self.patterns.iter().any(|r| r.is_match(&text)) {
            self.hider.hid(&line);
            // A progress line says nothing once the build is over; the summary
            // counts only the messages hidden.
            if !line.kind.is_progress() {
                self.hider.stats.hid(1);
            }
            return None;
        }
        Some(record.to_vec())
    }
}

/// Replaces every match of `regex` in a record with `replacement` (`$1`,
/// `${name}` expand). It sees the record as the stages before it left it,
/// escape sequences included, and leaves bytes that are not UTF-8 alone.
pub struct Rewrite {
    regex: BytesRegex,
    replacement: String,
    /// See [`Rewrite::under_pty`].
    pty: bool,
    given: Option<Line>,
}

impl Rewrite {
    pub fn new(regex: BytesRegex, replacement: String) -> Self {
        Self {
            regex,
            replacement,
            pty: false,
            given: None,
        }
    }

    /// For a PTY capture: a rewrite of a row of bazel's progress area
    /// ([`Line::drawn`]) stays one row: newlines in it become spaces, and one
    /// made longer is cut to the original's width, since bazel crops those
    /// rows to the terminal's and one that wrapped would leave curses' next
    /// redraw a row short.
    pub fn under_pty(mut self, pty: bool) -> Self {
        self.pty = pty;
        self
    }
}

impl LineProcessor for Rewrite {
    fn see(&mut self, line: &Line) {
        self.given = Some(line.clone());
    }

    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        let mut out = self
            .regex
            .replace_all(record, self.replacement.as_bytes())
            .into_owned();
        let in_progress_area = self.given.take().is_some_and(|l| l.drawn);
        if self.pty && in_progress_area {
            for b in &mut out {
                if *b == b'\n' || *b == b'\r' {
                    *b = b' ';
                }
            }
            if !keeps_its_rows(record, &out) {
                // Cut back rather than undone, so a redaction still holds.
                out = ansi::truncate(&out, ansi::width(record));
            }
        }
        Some(out)
    }
}

/// Whether a path could go on through `b`.
fn continues_path(b: u8) -> bool {
    super::links::is_path_byte(b)
}

/// Whether an absolute path after `b` is the rest of a longer path or a
/// URL (`file:///ws/x`) rather than one of its own. `@` is not, so a
/// response file (`@/ws/x.params`) is made relative as any path is.
fn joins_path(b: u8) -> bool {
    b != b'@' && continues_path(b)
}

/// An absolute path through an execroot, a sandbox's included, up to the
/// workspace's directory in it: `…/execroot/_main/`.
static EXECROOT: LazyLock<BytesRegex> = LazyLock::new(|| {
    BytesRegex::new(r#"(?:/[^/\s'"():,;=\[\]{}]+)+/execroot/[^/\s'"():,;=\[\]{}]+/"#).unwrap()
});

/// The sandbox part of a sandbox's execroot path: what makes it the
/// sandbox's rather than the output base's real execroot.
static SANDBOX: LazyLock<BytesRegex> =
    LazyLock::new(|| BytesRegex::new(r"/sandbox/[^/]+/\d+/execroot/").unwrap());

/// Shortens the paths bazel and the tools it runs print: a sandbox path, which
/// is gone once its action is, becomes the workspace path it mirrors
/// (`bazel-out/…`, `src/…`), and so does an execroot path to a source file;
/// so does a path under `root`, the workspace. Each is written relative to
/// `cwd` (`../src/…` from a subdirectory), or as an absolute path under
/// `root` when `cwd` is outside it. An execroot path into `bazel-out/` or
/// `external/` is left absolute: it exists as written, while `bazel-out/…`
/// only resolves through a convenience symlink, and `external/…` nowhere. A
/// sandbox path into either becomes the output base's execroot path, which
/// outlives the sandbox; one under a `--sandbox_base` elsewhere is left as
/// written, since the output base cannot be told from it.
/// Only a record's text is rewritten, never its escape sequences (an OSC 8
/// hyperlink's target, say).
pub struct RelativizePaths {
    root: Option<Vec<u8>>,
    /// When set, each path made relative is written as a link to the file
    /// it names ([`RelativizePaths::linking`]).
    link: Option<LinkTo>,
    /// Where `cwd` is: the workspace root, below it (its directories under
    /// the root), or outside it.
    cwd: Cwd,
    /// See [`RelativizePaths::under_pty`].
    pty: bool,
    given: Option<Line>,
}

/// How [`RelativizePaths`] links what it makes relative: on `host`, a
/// workspace path from an execroot under `workspace`, the workspace the
/// build runs in.
pub struct LinkTo {
    pub host: String,
    pub workspace: std::path::PathBuf,
}

/// Around a path made relative while its record is rewritten, before
/// [`RelativizePaths::linked`] writes it as a link: `OPEN file MID shown
/// CLOSE`. Control bytes a record's text never holds when linking is on.
const OPEN: u8 = 0x0e;
const MID: u8 = 0x0f;
const CLOSE: u8 = 0x10;

/// Where paths are made relative to: the workspace root, a directory below
/// it (its components), or one outside it.
enum Cwd {
    Root,
    Below(Vec<Vec<u8>>),
    Outside,
}

impl RelativizePaths {
    /// For a PTY capture: a line of bazel's progress area whose paths would
    /// come out longer (a short root below a deep cwd) is left as it was.
    pub fn under_pty(mut self, pty: bool) -> Self {
        self.pty = pty;
        self
    }

    /// This stage writing each path it makes relative as an OSC 8 link to
    /// the file it names, when that exists, for a terminal: the shown text
    /// alone no longer tells which file (`BUILD.bazel` from a package
    /// directory is that package's).
    pub fn linking(mut self, link: Option<LinkTo>) -> Self {
        self.link = link;
        self
    }

    /// `root` is the workspace directory and `cwd` the one paths are made
    /// relative to, `root` when None; trailing `/`s are ignored.
    pub fn new(root: Option<String>, cwd: Option<String>) -> Self {
        let trim = |p: String| p.trim_end_matches('/').to_string();
        let root = root.map(trim).filter(|r| !r.is_empty());
        let cwd = match (&root, cwd.map(trim)) {
            (Some(root), Some(cwd)) if cwd != *root => {
                match cwd.strip_prefix(&format!("{root}/")) {
                    Some(below) => {
                        Cwd::Below(below.split('/').map(|d| d.as_bytes().to_vec()).collect())
                    }
                    None => Cwd::Outside,
                }
            }
            _ => Cwd::Root,
        };
        Self {
            root: root.map(|r| (r + "/").into_bytes()),
            link: None,
            cwd,
            pty: false,
            given: None,
        }
    }

    /// For a workspace path whose rest (`src/a.cc:1 …`) follows: what to write
    /// in place of everything up to it, and how many bytes of `rest` that
    /// also stands for. From a subdirectory, the directories it shares with
    /// the path are dropped rather than walked back up (`a.cc`, not
    /// `../src/a.cc`, from `src/`).
    fn relative(&self, rest: &[u8]) -> (Vec<u8>, usize) {
        let (with, consumed) = match &self.cwd {
            Cwd::Root => (Vec::new(), 0),
            Cwd::Outside => (self.root.clone().unwrap_or_default(), 0),
            Cwd::Below(dirs) => {
                let mut shared = 0;
                let mut consumed = 0;
                for dir in dirs {
                    let at = &rest[consumed..];
                    if at.starts_with(dir) && at.get(dir.len()) == Some(&b'/') {
                        shared += 1;
                        consumed += dir.len() + 1;
                    } else {
                        break;
                    }
                }
                (b"../".repeat(dirs.len() - shared), consumed)
            }
        };
        // A path that comes to the cwd itself is `.`, not nothing.
        if with.is_empty() && !rest.get(consumed).is_some_and(|&b| continues_path(b)) {
            return (b".".to_vec(), consumed);
        }
        (with, consumed)
    }

    /// `with` shown in place of everything up to `rest`, which `consumed`
    /// bytes of `rest` also stand for, onto `out`; when linking, wrapped as
    /// the file it names, under `base` (`root`'s, or the build's workspace
    /// for `None`), for [`RelativizePaths::linked`]. How much of `rest` it
    /// stands for.
    fn shown(
        &self,
        base: Option<&[u8]>,
        with: Vec<u8>,
        rest: &[u8],
        consumed: usize,
        out: &mut Vec<u8>,
    ) -> usize {
        let Some(link) = &self.link else {
            out.extend(with);
            return consumed;
        };
        let mut end = rest
            .iter()
            .position(|&b| !super::links::is_path_byte(b))
            .unwrap_or(rest.len());
        while end > consumed && rest[end - 1] == b'.' {
            end -= 1;
        }
        if end <= consumed {
            out.extend(with);
            return consumed;
        }
        use std::os::unix::ffi::OsStrExt;
        out.push(OPEN);
        match base {
            Some(base) => out.extend_from_slice(base),
            None => {
                out.extend_from_slice(link.workspace.as_os_str().as_bytes());
                out.push(b'/');
            }
        }
        out.extend_from_slice(&rest[..end]);
        out.push(MID);
        out.extend(with);
        out.extend_from_slice(&rest[consumed..end]);
        out.push(CLOSE);
        end
    }

    fn text(&self, text: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len());
        let mut at = 0;
        for found in EXECROOT.find_iter(text) {
            if found.start() > 0 && joins_path(text[found.start() - 1]) {
                // Part of a longer path or a URL (`file:///…/execroot/…`).
                continue;
            }
            out.extend_from_slice(&text[at..found.start()]);
            let rest = &text[found.end()..];
            let sandbox = found.as_bytes().windows(9).any(|w| w == b"/sandbox/");
            let real = rest.starts_with(b"external/") || rest.starts_with(b"bazel-out/");
            at = found.end();
            if real && sandbox {
                // Gone with the sandbox, but the output base's own execroot
                // holds the same path.
                out.extend(SANDBOX.replace(found.as_bytes(), &b"/execroot/"[..]).iter());
            } else if real {
                out.extend_from_slice(found.as_bytes());
            } else {
                let (with, consumed) = self.relative(rest);
                at += self.shown(None, with, rest, consumed, &mut out);
            }
        }
        out.extend_from_slice(&text[at..]);
        match &self.root {
            Some(root) if !matches!(self.cwd, Cwd::Outside) => self.replace_root(&out, root),
            _ => out,
        }
    }

    /// `hay` with each occurrence of `root` that starts a path made
    /// relative: one not preceded by a byte a path could continue through
    /// but `@` (`file:///ws/x` keeps its root, `@/ws/x.params` does not). A
    /// path already made relative and wrapped to be linked is passed over.
    fn replace_root(&self, hay: &[u8], root: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(hay.len());
        let mut i = 0;
        while i < hay.len() {
            if self.link.is_some()
                && hay[i] == OPEN
                && let Some(len) = hay[i..].iter().position(|&b| b == CLOSE)
            {
                out.extend_from_slice(&hay[i..=i + len]);
                i += len + 1;
            } else if hay[i..].starts_with(root) && (i == 0 || !joins_path(hay[i - 1])) {
                let rest = &hay[i + root.len()..];
                let (with, consumed) = self.relative(rest);
                i += root.len() + self.shown(Some(root), with, rest, consumed, &mut out);
            } else {
                out.push(hay[i]);
                i += 1;
            }
        }
        out
    }
}

/// Whether `rewritten` takes no more terminal columns than `record`, so a
/// line of bazel's progress area, which bazel crops to the terminal's width,
/// does not wrap onto a row curses does not know of.
fn keeps_its_rows(record: &[u8], rewritten: &[u8]) -> bool {
    ansi::width(rewritten) <= ansi::width(record)
}

impl RelativizePaths {
    /// `record` with its paths made relative, each linked to its file when
    /// linking.
    fn relativized(&mut self, record: &[u8]) -> Vec<u8> {
        let in_progress_area = self.given.take().is_some_and(|l| l.drawn);
        let contains = |needle: &[u8]| record.windows(needle.len()).any(|w| w == needle);
        if !contains(b"/execroot/") && !self.root.as_deref().is_some_and(contains) {
            return record.to_vec();
        }
        // A record whose text holds the bytes the links are marked with
        // (never Bazel's) is relativized without them.
        let link = self
            .link
            .take_if(|_| record.iter().any(|&b| matches!(b, OPEN | MID | CLOSE)));
        let out = ansi::map_text(record, |text| self.text(text));
        let out = self.linked(&out);
        if link.is_some() {
            self.link = link;
        }
        if self.pty && in_progress_area && !keeps_its_rows(record, &out) {
            return record.to_vec();
        }
        out
    }

    /// `out` with each path [`RelativizePaths::shown`] wrapped written as an
    /// OSC 8 link to its file, or as shown when that does not exist.
    fn linked(&self, out: &[u8]) -> Vec<u8> {
        let Some(link) = &self.link else {
            return out.to_vec();
        };
        if !out.contains(&OPEN) {
            return out.to_vec();
        }
        use std::os::unix::ffi::OsStrExt;
        let mut linked = Vec::with_capacity(out.len() + 64);
        let mut i = 0;
        while i < out.len() {
            let parts = (out[i] == OPEN)
                .then(|| {
                    let mid = i + out[i..].iter().position(|&b| b == MID)?;
                    let close = mid + out[mid..].iter().position(|&b| b == CLOSE)?;
                    Some((mid, close))
                })
                .flatten();
            let Some((mid, close)) = parts else {
                linked.push(out[i]);
                i += 1;
                continue;
            };
            let file = std::path::Path::new(std::ffi::OsStr::from_bytes(&out[i + 1..mid]));
            let shown = &out[mid + 1..close];
            if file.exists() {
                linked.extend(super::links::osc8(
                    &super::links::file_url(&link.host, file),
                    shown,
                ));
            } else {
                linked.extend_from_slice(shown);
            }
            i = close + 1;
        }
        linked
    }
}

impl LineProcessor for RelativizePaths {
    fn see(&mut self, line: &Line) {
        self.given = Some(line.clone());
    }

    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        Some(self.relativized(record))
    }
}

/// Quoted text, labels, canonical repo names (`+ext+bar`, `_main~ext~bar`),
/// paths, hashes and numbers: what [`fingerprint`] masks, in that order of
/// preference.
static VARIABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"(?P<q>'[^']*'|"[^"]*"|`[^`]*`)"#,
        r"|(?P<l>@{0,2}[\w.~+-]*//[\w./~+@-]*(?::[\w./~+@=-]+)?)",
        r"|(?P<r>@{0,2}[\w.-]*[~+][\w.~+-]*)",
        r"|(?P<p>(?:[\w.~+-]*/)+[\w.~+-]*)",
        r"|(?P<h>\b(?:0x)?[0-9a-fA-F]{8,}\b)",
        r"|(?P<n>\d+)",
    ))
    .unwrap()
});

/// `text` with every label, repo name, path, number, hash and quoted string
/// replaced by a placeholder, so lines that differ only in those collapse
/// together.
pub fn fingerprint(text: &str) -> String {
    VARIABLE
        .replace_all(text, |caps: &regex::Captures| {
            match ["q", "l", "r", "p", "h", "n"]
                .into_iter()
                .find(|g| caps.name(g).is_some())
            {
                Some("q") => "'_'",
                Some("l") => "<label>",
                Some("r") => "<repo>",
                Some("p") => "<path>",
                Some("h") => "<hash>",
                _ => "#",
            }
        })
        .into_owned()
}

/// Templates [`CollapseSimilar`] tracks; a line of a template past this many
/// is shown.
pub const MAX_TEMPLATES: usize = 4096;

/// One collapsed template: the line first shown for it, how many like it
/// were hidden, and the file all of them are in once the console ends
/// ([`super::report::Stats::keep_groups`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collapsed {
    pub first: String,
    pub hidden: u64,
    pub file: Option<std::path::PathBuf>,
    /// Writing `file` failed part-way, so it holds fewer than all of them.
    pub incomplete: bool,
}

/// Shows the first message of each kind and hides the rest, body included.
///
/// A message's kind is the first `groups` pattern it matches, or, for a
/// message whose severity (`WARNING`, …) is in `severities`, its
/// [`fingerprint`]. Other messages pass. The kinds that hid something are in
/// [`Stats::collapsed`], for the report at the end of the console.
pub struct CollapseSimilar {
    severities: Vec<String>,
    groups: Vec<(String, Regex)>,
    hider: Hider,
    seen: HashMap<String, usize>,
    /// The kind whose message is being read, shown or hidden, and that
    /// message's body so far: each of its lines goes to the kind's file.
    following: Option<usize>,
    body: Body,
}

impl CollapseSimilar {
    pub fn new(
        severities: Vec<String>,
        groups: Vec<(String, Regex)>,
        pty: bool,
        stats: Arc<Stats>,
    ) -> Self {
        Self {
            severities,
            groups,
            hider: Hider::new(pty, stats),
            seen: HashMap::new(),
            following: None,
            body: Body::default(),
        }
    }

    fn kind(&self, text: &str) -> Option<String> {
        if let Some((id, _)) = self.groups.iter().find(|(_, r)| r.is_match(text)) {
            return Some(format!("group:{id}"));
        }
        let severity = text.split_once(": ")?.0;
        self.severities
            .iter()
            .any(|s| s == severity)
            .then(|| format!("fp:{}", fingerprint(text)))
    }
}

impl LineProcessor for CollapseSimilar {
    fn see(&mut self, line: &Line) {
        self.hider.given.see(line);
    }

    fn skipped(&mut self, line: &Line) {
        self.hider.skipped(line);
        // A record an earlier stage hid ends a message's body as one shown
        // would: the indented lines after it are someone else's.
        if self.following.is_some() && !self.body.swallows(line) {
            self.following = None;
        }
    }

    fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
        if let Some(slot) = self.following {
            if self.body.swallows(self.hider.given.peek(record)) {
                self.hider.stats.group_line(slot, record);
            } else {
                self.following = None;
            }
        }
        let (line, text) = self.hider.see(record)?;
        if !self.hider.target(&line) || line.kind != Kind::Header {
            return Some(record.to_vec());
        }
        let Some(kind) = self.kind(&text) else {
            return Some(record.to_vec());
        };
        let (slot, shown) = match self.seen.get(&kind) {
            Some(&slot) => {
                self.hider.stats.collapse(slot);
                self.hider.hid(&line);
                (slot, None)
            }
            None if self.seen.len() < MAX_TEMPLATES => {
                let slot = self.hider.stats.template(&text);
                self.seen.insert(kind, slot);
                (slot, Some(record.to_vec()))
            }
            None => return Some(record.to_vec()),
        };
        self.hider.stats.group_line(slot, record);
        self.following = Some(slot);
        self.body.open();
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::console::line::Classifier;
    use crate::console::line::tests::{bazel, unbazel};

    /// Run `records` through `p`, coloured as bazel colours its messages
    /// ([`bazel`]: a `~` marks a tool's line), and what it kept, uncoloured.
    fn feed(p: &mut dyn LineProcessor, records: &[&str]) -> Vec<String> {
        records
            .iter()
            .filter_map(|r| p.process(bazel(r).as_bytes()))
            .map(|b| unbazel(&b))
            .collect()
    }

    fn rx(p: &str) -> Regex {
        Regex::new(p).unwrap()
    }

    #[test]
    fn fingerprint_masks_what_varies() {
        assert_eq!(
            fingerprint(
                "WARNING: /ws/a/BUILD:12:3: target '//a:b' is deprecated, see https://x.dev/y"
            ),
            "WARNING: <path>:#:#: target '_' is deprecated, see https:<label>"
        );
        assert_eq!(
            fingerprint("DEBUG: @@rules_go~//go:def.bzl took 123ms (0xdeadbeef12)"),
            fingerprint("DEBUG: @@rules_py~//py:defs.bzl took 9ms (0x0123456789)")
        );
        assert_ne!(
            fingerprint("WARNING: foo is deprecated"),
            fingerprint("WARNING: bar is deprecated")
        );
        assert_eq!(
            fingerprint("DEBUG: Repo +ext+bar defined by rule http_archive"),
            fingerprint("DEBUG: Repo _main~ext~foo defined by rule http_archive")
        );
    }

    #[test]
    fn collapse_similar_shows_the_first_of_each_kind() {
        let stats = Arc::new(Stats::default());
        let mut p = CollapseSimilar::new(vec!["WARNING".into()], vec![], false, stats.clone());
        let out = feed(
            &mut p,
            &[
                "WARNING: errors encountered while analyzing target '//:g0'",
                "Analysis failed",
                "INFO: Analyzed 6 targets",
                "WARNING: errors encountered while analyzing target '//:g1'",
                "Analysis failed",
                "",
                "WARNING: errors encountered while analyzing target '//:g2'",
                "Analysis failed",
                "ERROR: errors encountered while analyzing target '//:g3'",
                "WARNING: something else",
            ],
        );
        assert_eq!(
            out,
            [
                "WARNING: errors encountered while analyzing target '//:g0'",
                "Analysis failed",
                "INFO: Analyzed 6 targets",
                "Analysis failed",
                "",
                "Analysis failed",
                "ERROR: errors encountered while analyzing target '//:g3'",
                "WARNING: something else",
            ],
            "a pipe marks no body, so an unindented line under a hidden message stays"
        );
        assert_eq!(stats.hidden(), 2);
        assert_eq!(
            stats.collapsed(),
            [Collapsed {
                first: "WARNING: errors encountered while analyzing target '//:g0'".into(),
                hidden: 2,
                file: None,
                incomplete: false,
            }]
        );
    }

    #[test]
    fn a_group_collapses_whatever_it_matches() {
        let stats = Arc::new(Stats::default());
        let mut p = CollapseSimilar::new(
            vec![],
            vec![("py".into(), rx("rules_python"))],
            false,
            stats.clone(),
        );
        let out = feed(
            &mut p,
            &[
                "DEBUG: rules_python: one",
                "INFO: rules_python: two",
                "DEBUG: other",
            ],
        );
        assert_eq!(out, ["DEBUG: rules_python: one", "DEBUG: other"]);
    }

    /// Under a PTY a message's record carries curses' erase prefix, and the
    /// progress line after it must survive.
    #[test]
    fn under_a_pty_progress_lines_are_never_hidden() {
        let stats = Arc::new(Stats::default());
        let mut p = CollapseSimilar::new(
            vec!["WARNING".into()],
            vec![("all".into(), rx("."))],
            true,
            stats,
        );
        let warn = "\x1b[1A\x1b[K\x1b[35mWARNING: \x1b[0mtarget '//:a' is odd";
        let out = feed(
            &mut p,
            &[
                warn,
                "\x1b[32mLoading:\x1b[0m 1 packages loaded",
                warn,
                "\x1b[32mLoading:\x1b[0m 1 packages loaded",
                "[3 / 9] Compiling a.cc",
            ],
        );
        assert_eq!(
            out,
            [
                warn,
                "\x1b[32mLoading:\x1b[0m 1 packages loaded",
                "\x1b[32mLoading:\x1b[0m 1 packages loaded",
                "[3 / 9] Compiling a.cc",
            ]
        );
    }

    #[test]
    fn templates_past_the_cap_are_shown() {
        let stats = Arc::new(Stats::default());
        let mut p = CollapseSimilar::new(vec!["WARNING".into()], vec![], false, stats);
        for i in 0..MAX_TEMPLATES {
            let line = format!("WARNING: {}", "x".repeat(i % 50) + &"y".repeat(i / 50));
            assert!(p.process(line.as_bytes()).is_some());
        }
        let extra = b"WARNING: past the cap";
        assert!(p.process(extra).is_some());
        assert!(
            p.process(extra).is_some(),
            "an untracked kind is not collapsed"
        );
    }

    #[test]
    fn drop_lines_hides_a_message_and_its_body() {
        let stats = Arc::new(Stats::default());
        let mut p = DropLines::new(vec![rx("^DEBUG: Rule")], false, stats.clone());
        let out = feed(
            &mut p,
            &[
                "DEBUG: Rule 'x' indicated that a canonical reproducible form can be obtained",
                "  more of it",
                "DEBUG: kept",
                "  kept body",
            ],
        );
        assert_eq!(out, ["DEBUG: kept", "  kept body"]);
        assert_eq!(stats.hidden(), 2);
    }

    /// In a pipe bazel writes a blank line after its `Analyzing:` lines; with
    /// those dropped it would be all that is left of them.
    #[test]
    fn a_dropped_pipe_progress_line_takes_the_blank_after_it() {
        let stats = Arc::new(Stats::default());
        let mut p = DropLines::new(vec![rx("^Analyzing:")], false, stats.clone());
        let out = feed(
            &mut p,
            &[
                "\x1b[32mAnalyzing:\x1b[0m 2 targets",
                "",
                "INFO: Analyzed 2 targets",
                "",
            ],
        );
        assert_eq!(out, ["INFO: Analyzed 2 targets", ""]);
        assert_eq!(stats.hidden(), 0);
    }

    #[test]
    fn drop_lines_never_hides_a_blank_line() {
        let stats = Arc::new(Stats::default());
        let mut p = DropLines::new(vec![rx("^$"), rx("^Loading")], true, stats);
        assert_eq!(feed(&mut p, &["", "Loading: 1"]), ["", "Loading: 1"]);
    }

    #[test]
    fn rewrite_replaces_every_match() {
        let mut p = Rewrite::new(BytesRegex::new(r"(token=)\S+").unwrap(), "${1}***".into());
        assert_eq!(
            feed(&mut p, &["a token=abc b token=def", "plain"]),
            ["a token=*** b token=***", "plain"]
        );
    }

    #[test]
    fn relativize_paths_strips_execroots_and_the_workspace() {
        let mut p = RelativizePaths::new(Some("/Users/me/ws/".into()), None);
        let out = feed(
            &mut p,
            &[
                "src/a.ts(3,4): /private/var/tmp/_bazel_me/abc/sandbox/darwin-sandbox/12/execroot/_main/src/a.ts:3:4 error",
                "\x1b[33mDEBUG: \x1b[0m/Users/me/ws/BUILD.bazel:6:6: noisy",
                "/home/u/.cache/bazel/_bazel_u/h/execroot/_main/bazel-out/k8-fastbuild/bin/a.o",
                "/home/u/.cache/bazel/_bazel_u/h/execroot/_main/external/rules_x/x.bzl:1",
                "/tmp/_bazel_u/h/sandbox/linux-sandbox/7/execroot/_main/bazel-out/k8/bin/b.o",
                "(/b/execroot/ws/pkg/f.cc) and '/c/execroot/ws/g.cc'",
                "/Users/me/wsx/BUILD untouched",
                "file:///Users/me/ws/a and x/Users/me/ws/b",
            ],
        );
        assert_eq!(
            out,
            [
                "src/a.ts(3,4): src/a.ts:3:4 error",
                "\x1b[33mDEBUG: \x1b[0mBUILD.bazel:6:6: noisy",
                "/home/u/.cache/bazel/_bazel_u/h/execroot/_main/bazel-out/k8-fastbuild/bin/a.o",
                "/home/u/.cache/bazel/_bazel_u/h/execroot/_main/external/rules_x/x.bzl:1",
                "/tmp/_bazel_u/h/execroot/_main/bazel-out/k8/bin/b.o",
                "(pkg/f.cc) and 'g.cc'",
                "/Users/me/wsx/BUILD untouched",
                "file:///Users/me/ws/a and x/Users/me/ws/b",
            ]
        );
    }

    /// With `--show_timestamps` every line starts `(hh:mm:ss) `; a hidden
    /// message must still end at the next header.
    #[test]
    fn timestamped_messages_are_classified_by_their_text() {
        let stats = Arc::new(Stats::default());
        let mut p = DropLines::new(vec![rx("^DEBUG: Rule")], true, stats.clone());
        let out = feed(
            &mut p,
            &[
                "(12:00:00) DEBUG: Rule 'x' indicated",
                "(12:00:01) [4 / 5] Compiling a.cc; 1s local",
                "    Compiling b.cc",
                "(12:00:02) ERROR: broke",
            ],
        );
        assert_eq!(
            out,
            [
                "(12:00:01) [4 / 5] Compiling a.cc; 1s local",
                "    Compiling b.cc",
                "(12:00:02) ERROR: broke",
            ]
        );
        let stats = Arc::new(Stats::default());
        let mut p = CollapseSimilar::new(vec!["WARNING".into()], vec![], false, stats);
        assert_eq!(
            feed(
                &mut p,
                &["(12:00:00) WARNING: a 1", "(12:00:05) WARNING: a 2"]
            ),
            ["(12:00:00) WARNING: a 1"]
        );
    }

    /// The agent profile drops progress lines in a pipe; what follows one is
    /// not its body.
    #[test]
    fn a_dropped_progress_line_takes_nothing_with_it() {
        let stats = Arc::new(Stats::default());
        let mut p = DropLines::new(vec![rx(r"^\[[\d,]+ / [\d,]+\] ")], false, stats.clone());
        let out = feed(
            &mut p,
            &[
                "\x1b[32m[4 / 5]\x1b[0m 2 actions running",
                "    Compiling a.cc; 1s local",
                "Target //:x up-to-date:",
                "  bazel-bin/x",
                "INFO: Elapsed time: 1s",
            ],
        );
        assert_eq!(
            out,
            [
                "    Compiling a.cc; 1s local",
                "Target //:x up-to-date:",
                "  bazel-bin/x",
                "INFO: Elapsed time: 1s"
            ]
        );
        assert_eq!(
            stats.hidden(),
            0,
            "a progress line is not a message the summary counts"
        );
    }

    /// Under a PTY the indented lines of the progress area are as off limits
    /// as its first.
    #[test]
    fn under_a_pty_the_whole_progress_area_is_kept() {
        let stats = Arc::new(Stats::default());
        let mut p = DropLines::new(vec![rx("Compiling")], true, stats.clone());
        let records = [
            "\x1b[32m[3 / 9]\x1b[0m 2 actions running",
            "    Compiling a.cc; 1s local",
            "    Compiling b.cc; 1s local",
        ];
        assert_eq!(feed(&mut p, &records), records);
        assert_eq!(stats.hidden(), 0);
    }

    #[test]
    fn under_a_pty_a_rewritten_progress_line_stays_one_line() {
        let mut p = Rewrite::new(BytesRegex::new("tok").unwrap(), "a\nb".into()).under_pty(true);
        let mut lines = Classifier::default();
        let record = b"\x1b[32m[1 / 2]\x1b[0m Fetching tok";
        p.see(&lines.classify(record));
        assert_eq!(
            p.process(record).as_deref(),
            Some(&b"\x1b[32m[1 / 2]\x1b[0m Fetching a b"[..])
        );
    }

    /// A short root below a deep cwd makes `../../../` of `/w/`: under a PTY
    /// a progress line is left as bazel cropped it rather than wrap.
    #[test]
    fn under_a_pty_relativizing_never_widens_a_progress_line() {
        let record = b"\x1b[32m[1 / 2]\x1b[0m Running /w/tools/gen.sh";
        let run = |pty: bool| {
            let mut p =
                RelativizePaths::new(Some("/w".into()), Some("/w/a/b/c".into())).under_pty(pty);
            p.see(&Classifier::default().classify(record));
            p.process(record).unwrap()
        };
        assert_eq!(run(true), record.to_vec());
        assert_eq!(
            String::from_utf8(run(false)).unwrap(),
            "\x1b[32m[1 / 2]\x1b[0m Running ../../../tools/gen.sh"
        );
    }

    #[test]
    fn an_execroot_path_in_a_file_url_is_left_whole() {
        let mut p = RelativizePaths::new(Some("/ws".into()), None);
        let url = b"see file:///tmp/_bazel_u/abc/execroot/_main/src/a.cc";
        assert_eq!(p.process(url).as_deref(), Some(&url[..]));
    }

    /// Cut to the width bazel cropped it to, so a redaction still holds.
    #[test]
    fn under_a_pty_a_progress_line_is_never_made_longer() {
        let mut p = Rewrite::new(BytesRegex::new("genrule").unwrap(), "genrule action".into())
            .under_pty(true);
        let record = b"\x1b[32m[1 / 2]\x1b[0m Executing genrule //:g";
        p.see(&Classifier::default().classify(record));
        assert_eq!(
            p.process(record).as_deref(),
            Some(&b"\x1b[32m[1 / 2]\x1b[0m Executing genrule acti"[..])
        );

        let mut redact = Rewrite::new(BytesRegex::new("u:[^@]*@").unwrap(), "<redacted>@".into())
            .under_pty(true);
        let fetching = b"\x1b[32mFetching:\x1b[0m https://u:t@x.dev/a; 1s";
        redact.see(&Classifier::default().classify(fetching));
        let out = redact.process(fetching).unwrap();
        assert_eq!(ansi::width(&out), ansi::width(fetching));
        assert!(!ansi::text(&out).contains("u:t@"), "{}", ansi::text(&out));
    }

    /// A marker of two columns and a space in place of three indent spaces
    /// (the Theme plugin's) keeps a progress row whole under a PTY; one in
    /// place of two would cost the row its last column.
    #[test]
    fn a_wide_marker_in_place_of_three_spaces_keeps_the_row() {
        let row = b"    Compiling a.cc; 12s remote";
        let marked = |spaces: &str| {
            let mut p = Rewrite::new(
                BytesRegex::new(&format!(r"^(\s*)\s{spaces}(Compiling)")).unwrap(),
                "${1}🔨 ${2}".into(),
            )
            .under_pty(true);
            let mut line = Classifier::default().classify(row);
            line.drawn = true;
            p.see(&line);
            p.process(row).unwrap()
        };
        let three = marked("{3}");
        assert_eq!(ansi::text(&three), " 🔨 Compiling a.cc; 12s remote");
        assert_eq!(ansi::width(&three), ansi::width(row));
        let two = marked("{2}");
        assert!(ansi::text(&two).ends_with("remot"), "{}", ansi::text(&two));
    }

    #[test]
    fn rewriting_leaves_bytes_that_are_not_utf8_alone() {
        let mut p = RelativizePaths::new(Some("/ws".into()), None);
        assert_eq!(
            p.process(b"/ws/a \xff\xfe bytes").as_deref(),
            Some(&b"a \xff\xfe bytes"[..])
        );
        let mut p = Rewrite::new(BytesRegex::new("a").unwrap(), "b".into());
        assert_eq!(p.process(b"a\xff").as_deref(), Some(&b"b\xff"[..]));
    }

    #[test]
    fn relativize_paths_leaves_escape_sequences_alone() {
        let mut p = RelativizePaths::new(Some("/ws".into()), None);
        let link = b"\x1b]8;;file:///ws/a.cc\x1b\\/ws/a.cc\x1b]8;;\x1b\\";
        assert_eq!(
            p.process(link).as_deref(),
            Some(&b"\x1b]8;;file:///ws/a.cc\x1b\\a.cc\x1b]8;;\x1b\\"[..])
        );
    }

    /// A tool's output with a blank line in it, from three actions: nothing
    /// in it is bazel's, so nothing is collapsed.
    #[test]
    fn action_output_is_never_collapsed() {
        for (header, pty) in [
            (
                "\x1b[1A\x1b[K\x1b[32mINFO: \x1b[0mFrom Executing genrule //:g:",
                true,
            ),
            ("INFO: From Executing genrule //:g:", false),
        ] {
            let stats = Arc::new(Stats::default());
            let mut p = CollapseSimilar::new(vec!["WARNING".into()], vec![], pty, stats.clone());
            let action = [
                header,
                "~line A",
                "",
                "~WARNING: same thing",
                "~tail of output",
            ];
            let records: Vec<&str> = action
                .iter()
                .chain(&action)
                .chain(&action)
                .copied()
                .collect();
            let shown: Vec<String> = records
                .iter()
                .map(|r| r.trim_start_matches('~').to_string())
                .collect();
            assert_eq!(feed(&mut p, &records), shown, "pty: {pty}");
            assert_eq!(stats.hidden(), 0);
        }
    }

    #[test]
    fn back_to_back_coloured_messages_end_a_hidden_one() {
        let stats = Arc::new(Stats::default());
        let mut p = CollapseSimilar::new(vec!["WARNING".into()], vec![], true, stats.clone());
        let warn = "\x1b[35mWARNING: \x1b[0msame thing";
        let out = feed(
            &mut p,
            &[
                warn,
                warn,
                "\x1b[32mINFO: \x1b[0mFrom Executing genrule //:chatty:",
                "chatter",
                "\x1b[32mINFO: \x1b[0mBuild completed successfully",
            ],
        );
        assert_eq!(
            out,
            [
                warn,
                "\x1b[32mINFO: \x1b[0mFrom Executing genrule //:chatty:",
                "chatter",
                "\x1b[32mINFO: \x1b[0mBuild completed successfully",
            ]
        );
        assert_eq!(stats.hidden(), 1);
    }

    /// A pattern names messages; a line of action output that happens to
    /// match it is not one, and hides nothing.
    #[test]
    fn a_pattern_never_hides_action_output() {
        let stats = Arc::new(Stats::default());
        let mut p = DropLines::new(vec![rx("^DEBUG: Rule")], true, stats.clone());
        let records = [
            "\x1b[32mINFO: \x1b[0mFrom Executing genrule //:g:",
            "~DEBUG: Rule 'x' indicated that a canonical",
            "~  tool line 1",
            "~tool line 2",
        ];
        let shown: Vec<&str> = records.iter().map(|r| r.trim_start_matches('~')).collect();
        assert_eq!(feed(&mut p, &records), shown);
        assert_eq!(stats.hidden(), 0);
    }

    #[test]
    fn relativize_paths_keeps_neighbouring_text() {
        let mut p = RelativizePaths::new(Some("/ws".into()), None);
        let run =
            |p: &mut RelativizePaths, l: &[u8]| String::from_utf8(p.process(l).unwrap()).unwrap();
        assert_eq!(
            run(
                &mut p,
                b"srcs=[/o/execroot/_main/src/a.cc,/o/execroot/_main/src/b.cc]"
            ),
            "srcs=[src/a.cc,src/b.cc]"
        );
        assert_eq!(run(&mut p, b"cd /ws/ && make"), "cd . && make");
        assert_eq!(run(&mut p, b"cd /o/execroot/_main/ && ls"), "cd . && ls");
        let mut below = RelativizePaths::new(Some("/ws".into()), Some("/ws/pkg".into()));
        assert_eq!(run(&mut below, b"in /ws/pkg/ now"), "in . now");
        assert_eq!(run(&mut below, b"in /ws/pkg/a.cc now"), "in a.cc now");
        assert_eq!(
            run(
                &mut p,
                b"/o/sandbox/darwin-sandbox/7/execroot/_main/external/rules_x/x.h:1"
            ),
            "/o/execroot/_main/external/rules_x/x.h:1"
        );
    }

    #[test]
    fn a_response_files_path_is_made_relative() {
        let mut p = RelativizePaths::new(Some("/ws".into()), None);
        let run = |p: &mut RelativizePaths, s: &str| {
            String::from_utf8(p.process(s.as_bytes()).unwrap()).unwrap()
        };
        assert_eq!(
            run(&mut p, "argfile @/ws/pkg/x.params"),
            "argfile @pkg/x.params"
        );
        assert_eq!(
            run(
                &mut p,
                "argfile @/o/sandbox/darwin-sandbox/3/execroot/_main/bazel-out/bin/x.params"
            ),
            "argfile @/o/execroot/_main/bazel-out/bin/x.params"
        );
        assert_eq!(run(&mut p, "file:///ws/a stays"), "file:///ws/a stays");
    }

    #[test]
    fn shift_bytes_in_a_tools_output_hide_no_path() {
        let line = "so \x0e /ws/pkg/BUILD.bazel \x10 and /ws/pkg/BUILD.bazel";
        let want = "so \x0e pkg/BUILD.bazel \x10 and pkg/BUILD.bazel";
        let mut plain = RelativizePaths::new(Some("/ws".into()), None);
        assert_eq!(
            String::from_utf8(plain.process(line.as_bytes()).unwrap()).unwrap(),
            want
        );
        let mut linking = RelativizePaths::new(Some("/ws".into()), None).linking(Some(LinkTo {
            host: String::new(),
            workspace: "/ws".into(),
        }));
        assert_eq!(
            String::from_utf8(linking.process(line.as_bytes()).unwrap()).unwrap(),
            want,
            "made relative without links"
        );
    }

    #[test]
    fn paths_are_relative_to_the_cwd() {
        let line = b"ERROR: /ws/bad/BUILD:3:1: x; /t/sandbox/s/1/execroot/_main/src/a.cc:1";
        let run = |cwd: &str| {
            let mut p = RelativizePaths::new(Some("/ws".into()), Some(cwd.into()));
            String::from_utf8(p.process(line).unwrap()).unwrap()
        };
        assert_eq!(run("/ws"), "ERROR: bad/BUILD:3:1: x; src/a.cc:1");
        assert_eq!(
            run("/ws/pkg/sub/"),
            "ERROR: ../../bad/BUILD:3:1: x; ../../src/a.cc:1"
        );
        assert_eq!(
            run("/elsewhere"),
            "ERROR: /ws/bad/BUILD:3:1: x; /ws/src/a.cc:1"
        );
    }
}
