use std::fs::File;
use std::os::fd::FromRawFd;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::fd::{STALL_LIMIT, writable, write_patiently};

/// The dump's line for the older lines it leaves out (`{n}` their count),
/// naming the task to show them, as the Dashboard's own release does.
pub(super) fn older_lines_note() -> String {
    let id = crate::out::output_ansi_path()
        .as_deref()
        .and_then(std::path::Path::parent)
        .and_then(crate::engine::history::entry_task_id);
    match id {
        Some(id) => format!("… {{n}} older lines: aspect history show {id}"),
        None => "… {n} older lines: aspect history show".to_string(),
    }
}

/// Where `output.ansi` stood when the screen was taken (or came back from a
/// suspend): what a dump on leaving writes from, the main screen showing
/// what came before. None once dumped, or with no screen taken.
static DUMP_MARK: Mutex<Option<u64>> = Mutex::new(None);

/// Whether the screen given back last had its dump written, or is having it
/// written (set as the mark is taken): by its release ([`release`] with a
/// dump), by a suspend still in force, or by a dump of its own. A later
/// [`dump_output`] says so rather than write it again. Cleared when a screen
/// is taken or comes back from a suspend.
pub(super) static DUMPED: AtomicBool = AtomicBool::new(false);

/// The dump's mark from here on (at a take, or back from a suspend): what
/// follows is dumped on leaving, and nothing of it is dumped yet.
pub(super) fn set_dump_mark() {
    *DUMP_MARK.lock().unwrap() = crate::out::output_ansi_mark();
    DUMPED.store(false, Ordering::SeqCst);
}

/// How long a dump that takes stderr's turn itself waits for a writer that
/// left stderr mid-line to end its line.
const LINE_END_WAIT: Duration = Duration::from_millis(200);

/// How long a dump waits for the terminal to take writes before it flushes
/// std's stdout into it: one stopped (Ctrl+S) is not flushed into.
const FLUSH_WAIT: Duration = Duration::from_millis(200);

/// The most lines a dump writes, the older counted on a line before them;
/// `std.term.dump_lines` to AXL.
pub(crate) const DUMP_LINES: usize = 20000;

/// Whether a dump would write anything: a mark, and the task's files.
pub(super) fn dump_ready() -> bool {
    DUMP_MARK.lock().unwrap().is_some() && crate::out::output_ansi_mark().is_some()
}

/// Write the task's output since the mark ([`DUMP_MARK`]) to stderr, as
/// `output.ansi` keeps it, its links made again as the CLI's own
/// lines make them (the file keeps colour, not links; Bazel's lines are kept
/// before their URLs are written), but for a line holding a URL already
/// where the terminal shows them as text (the CLI's own lines write
/// theirs). Its newest `max_lines`,
/// after a line `older` gives, `{n}` the count of those it leaves out.
/// An unended last line (the task's, the console's, or a stray writer's the
/// capture copied) is written as far as it goes, its end following on the
/// terminal. Where the terminal is left mid-line, the dump starts on a line
/// of its own: no row is erased, so a line the CLI left unended shows its
/// start again in the dump. Not copied to the output
/// files, which hold it; what stderr takes nothing of for `STALL_LIMIT` is
/// dropped. Whether the screen's dump is written: by this call, or already
/// by the release or suspend that gave the screen back ([`DUMPED`]); never
/// twice, nor without a mark or the task's files.
pub(crate) fn dump_output(max_lines: usize, older: &str) -> bool {
    if DUMPED.load(Ordering::SeqCst) {
        return true;
    }
    dump_output_turned(max_lines, older, false)
}

/// A pause between a dump reading `output.ansi` and writing it, from
/// `ASPECT_TEST_DUMP_DELAY_MS` (read once, at most 5s): for tests, to widen
/// the window a writer not held off would land in.
fn dump_test_delay() {
    static DELAY: std::sync::OnceLock<Option<Duration>> = std::sync::OnceLock::new();
    let delay = DELAY.get_or_init(|| {
        std::env::var("ASPECT_TEST_DUMP_DELAY_MS")
            .ok()
            .and_then(|ms| ms.parse::<u64>().ok())
            .map(|ms| Duration::from_millis(ms.min(5000)))
    });
    if let Some(delay) = delay {
        std::thread::sleep(*delay);
    }
}

/// [`dump_output`], with stderr's target already held by the caller
/// (`held`): for a release or a suspend, which holds the targets from giving
/// the descriptors back to the end of the dump, so no line printed meanwhile
/// shows both live and in it. Where giving the descriptors back gave up on
/// stderr's (a writer stalled on it), `held` is false and the dump takes
/// stderr's turn itself, as a dump of its own does; one given up on stdout's
/// alone leaves `held` true.
pub(super) fn dump_output_turned(max_lines: usize, older: &str, held: bool) -> bool {
    // Said to be dumped as the mark is taken, under its lock: a release or
    // suspend racing this one sees either the mark or that it is dumped.
    let mark = {
        let mut mark = DUMP_MARK.lock().unwrap();
        let taken = mark.take();
        if taken.is_some() {
            DUMPED.store(true, Ordering::SeqCst);
        }
        taken
    };
    let Some(mark) = mark else {
        return false;
    };
    let Some((text, _)) = crate::out::output_ansi_since(mark) else {
        DUMPED.store(false, Ordering::SeqCst);
        return false;
    };
    dump_test_delay();
    let relink = crate::out::terminal_links();
    let as_text = crate::out::links_as_text();
    let before = crate::out::links_written();
    let out = dump_text(&String::from_utf8_lossy(&text), max_lines, older, |line| {
        if relink && !(as_text && line.contains("://")) {
            crate::out::linked(line)
        } else {
            line.to_string()
        }
    });
    DUMP_WRITTEN.fetch_add(crate::out::links_written() - before, Ordering::SeqCst);
    // Not through `std::io::stderr()`, whose lock a writer stalled on a
    // stderr nobody reads may hold; the target's lock keeps every other
    // writer out of the dump instead, stdout's too where they share it.
    // Bounded as the replay is.
    // SAFETY: fd 2 stays open; ManuallyDrop never closes it.
    let mut err = std::mem::ManuallyDrop::new(unsafe { File::from_raw_fd(2) });
    let turn =
        (!held).then(|| axl_types::terminal::turn_at_line_end(2, STALL_LIMIT, LINE_END_WAIT));
    let turned = held || turn.as_ref().is_some_and(|t| t.is_some());
    // What std's stdout holds of an unended line goes out first where it
    // shares the terminal, so it is not left to surface after everything.
    // Only with the target held (stderr's is stdout's here), else a writer
    // stalled on it holds std's lock too; and only while the terminal takes
    // writes, so one stopped (Ctrl+S) never blocks it with every target held.
    // SAFETY: isatty on fd 1.
    if turned
        && unsafe { libc::isatty(1) } == 1
        && axl_types::terminal::one_target(1, 2)
        && writable(1, FLUSH_WAIT)
    {
        use std::io::Write as _;
        let _ = std::io::stdout().lock().flush();
    }
    // The terminal left mid-line (a line the CLI, a child or a stray
    // writer left unended): the dump starts a line of its own. That line's
    // start shows again in the dump where it is the CLI's.
    if axl_types::terminal::mid_line(2) {
        write_patiently(&mut err, b"\n");
    }
    write_patiently(&mut err, out.as_bytes());
    axl_types::terminal::wrote(2, out.as_bytes());
    true
}

/// The URLs the dumps have written out beside their links ([`dump_output`]):
/// a build's console writes none out while a screen holds the terminal, so
/// these are the ones a terminal that shows them as text has in its
/// scrollback.
pub(super) static DUMP_WRITTEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What [`dump_output`] writes of `text`: its newest `max_lines` lines,
/// each through `link` without its end, after a line `older` gives (`{n}`
/// the count left out) when any were.
fn dump_text(text: &str, max_lines: usize, older: &str, link: impl Fn(&str) -> String) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let skip = lines.len().saturating_sub(max_lines);
    let mut out = String::with_capacity(text.len() + 64);
    if skip > 0 && !older.is_empty() {
        out.push_str(&older.replace("{n}", &skip.to_string()));
        out.push('\n');
    }
    for line in &lines[skip..] {
        match line.strip_suffix('\n') {
            Some(body) => {
                out.push_str(&link(body));
                out.push('\n');
            }
            None => out.push_str(&link(line)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    /// What a dump re-links is counted when it writes a file's URL out (the
    /// macOS Terminal, where the links tip is due), and not when it only
    /// links it (iTerm2, VS Code, Cursor), nor for text naming no file.
    #[test]
    fn a_dump_counts_the_urls_it_writes_out() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        for (written, want) in [(true, 1), (false, 0)] {
            let count = bazel::console::links::WrittenCount::default();
            let mut linker = bazel::console::links::Linker::new(vec![dir.path().to_path_buf()])
                .with_files(
                    bazel::console::links::FileLinks::new(
                        bazel::console::links::LinkFormat::parse("vscode").unwrap(),
                    )
                    .written(written)
                    .counted(count.clone()),
                );
            linker.link_written(b"see a.txt:1");
            linker.link_written(b"nothing to link here");
            assert_eq!(count.get(), want, "written out: {written}");
        }
    }

    /// A dump keeps the newest lines, counts the rest on a line before
    /// them, and links each line without its end.
    #[test]
    fn a_dump_keeps_the_newest_lines_linked() {
        let text = "one\ntwo\nthree\nfour";
        assert_eq!(
            super::dump_text(text, 2, "… {n} older", |l| format!("[{l}]")),
            "… 2 older\n[three]\n[four]"
        );
        assert_eq!(
            super::dump_text(text, 9, "… {n} older", |l| l.to_string()),
            text
        );
        assert_eq!(super::dump_text("a\nb\n", 1, "", |l| l.to_string()), "b\n");
    }
}
