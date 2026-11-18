//! Bazel's expanded execution log formats, written from a compact log: the
//! binary `--execution_log_binary_file` (length-delimited `SpawnExec`s) and
//! the JSON `--execution_log_json_file`, as Bazel's own converter writes them.
//!
//! Bazel writes one execution log format per build. When the CLI keeps the
//! compact log, a binary or JSON log the caller asked for is written here
//! instead (see [`request`]), from that compact log once Bazel has finished
//! it: each spawn is [`reconstruct`]ed, put in Bazel's [`sort`] order when the
//! caller's `--execution_log_sort` says so, and written in the format asked
//! for.

mod json;
mod reconstruct;
mod sort;

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use axl_proto::tools::protos::{ExecLogEntry, SpawnExec};
use prost::Message;

use crate::delimited;
use reconstruct::Reconstructor;

/// An expanded execution log format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// `--execution_log_binary_file`: length-delimited `SpawnExec` messages.
    Binary,
    /// `--execution_log_json_file`: `SpawnExec` messages as JSON, one after
    /// another.
    Json,
}

impl Format {
    /// The flag that asks Bazel for this format.
    pub fn flag(self) -> &'static str {
        match self {
            Format::Binary => "--execution_log_binary_file",
            Format::Json => "--execution_log_json_file",
        }
    }
}

/// What a build's flags ask of the binary and JSON formats.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    /// Neither.
    None,
    /// Files the CLI writes from the compact log, each in its format, sorted
    /// or not. `off` turns Bazel's own formats off, after the caller's flags.
    Files {
        files: Vec<(Format, PathBuf)>,
        sorted: bool,
        off: Vec<String>,
    },
    /// A format left to Bazel, which then writes no compact log: Bazel 10's
    /// default location or remote stream (an empty or `true` value), a Bazel
    /// older than 7.4, or one whose version cannot be told.
    Bazels,
}

/// What `flags` ask of the binary and JSON formats for `verb` on `version`.
/// Paths are placed as the compact log's are (see
/// [`super::requested_path`]), against `directory` and `workspace`.
///
/// Up to Bazel 9 the flags take a path, and an empty one turns them off; from
/// 10 they take a boolean too. Both formats at once, which Bazel refuses, are
/// both written.
pub fn request(
    flags: &[String],
    verb: &str,
    directory: Option<&str>,
    workspace: Option<&Path>,
    version: Option<&semver::Version>,
) -> Request {
    // `None` when the version cannot be told, which could read a value either
    // way, so a format asked for is left to Bazel.
    let before_10 =
        version.map(|v| crate::rc::constraint_matches("<10.0.0", Some(v)).unwrap_or(false));
    let mut files = Vec::new();
    for format in [Format::Binary, Format::Json] {
        let word = crate::rc::last_word(flags, verb, |t| {
            crate::rc::bool_token(t, format.flag()).map(|w| w.map_err(str::to_owned))
        });
        let value = match (before_10, word) {
            (_, None) => continue,
            (Some(true), _) => match crate::rc::last_flag_value(flags, format.flag(), verb) {
                Some(value) if !value.is_empty() => value,
                _ => continue,
            },
            (_, Some(Ok(false))) => continue,
            (Some(false), Some(Err(value))) if !value.is_empty() => value,
            _ => return Request::Bazels,
        };
        files.push((format, super::place(&value, directory, workspace)));
    }
    if files.is_empty() {
        return Request::None;
    }
    if !super::knows_compact_file_flag(version) {
        return Request::Bazels;
    }
    let off = files
        .iter()
        .map(|(format, _)| match before_10 {
            Some(true) => format!("{}=", format.flag()),
            _ => format!("{}=false", format.flag()),
        })
        .collect();
    let sorted = crate::rc::last_word(flags, verb, |t| {
        crate::rc::bool_token(t, SORT_FLAG).and_then(Result::ok)
    })
    .unwrap_or(true);
    Request::Files { files, sorted, off }
}

/// The flag saying whether a binary or JSON log is sorted, which it is by
/// default.
const SORT_FLAG: &str = "--execution_log_sort";

/// Hand each spawn in the zstd-compressed compact log `compact` to `each`,
/// in log order.
fn each_spawn(
    compact: impl Read,
    mut each: impl FnMut(SpawnExec) -> io::Result<()>,
) -> io::Result<()> {
    let mut decoder = zstd::Decoder::new(compact)?;
    let mut reconstructor = Reconstructor::default();
    let mut buf = Vec::new();
    while let Some(header) = delimited::next_frame(&mut decoder, &mut buf)? {
        let entry = ExecLogEntry::decode(&buf[..header.len])
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if let Some(spawn) = reconstructor.push(entry)? {
            each(spawn)?;
        }
    }
    Ok(())
}

/// Write `spawn` to `out` in `format`.
fn write(spawn: &SpawnExec, format: Format, out: &mut impl Write) -> io::Result<()> {
    match format {
        Format::Binary => out.write_all(&spawn.encode_length_delimited_to_vec()),
        Format::Json => out.write_all(json::spawn_exec(spawn).as_bytes()),
    }
}

/// Write the compact log at `compact` to `out` as `format`: in Bazel's
/// stable order when `sorted`, which holds every spawn in memory to sort
/// them, as Bazel does, and otherwise each as it is read.
pub fn convert(compact: &Path, out: &Path, format: Format, sorted: bool) -> io::Result<()> {
    let compact = io::BufReader::new(File::open(compact)?);
    let mut file = BufWriter::new(File::create(out)?);
    if sorted {
        let mut spawns = Vec::new();
        each_spawn(compact, |spawn| {
            spawns.push(spawn);
            Ok(())
        })?;
        for spawn in sort::stable_sort(spawns) {
            write(&spawn, format, &mut file)?;
        }
    } else {
        each_spawn(compact, |spawn| write(&spawn, format, &mut file))?;
    }
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The logs one Bazel version wrote for the fixture build, each format
    /// written by Bazel itself: `testdata/README.md` says how.
    struct Fixture {
        compact: &'static [u8],
        binary: &'static [u8],
        json: &'static str,
    }

    macro_rules! fixture {
        ($version:literal) => {
            Fixture {
                compact: include_bytes!(concat!("testdata/v", $version, "/compact.zstd")),
                binary: include_bytes!(concat!("testdata/v", $version, "/binary.binpb")),
                json: include_str!(concat!("testdata/v", $version, "/log.json")),
            }
        };
    }

    const FIXTURES: [(&str, Fixture); 3] = [
        ("7.4.1", fixture!("7.4.1")),
        ("8.8.2", fixture!("8.8.2")),
        ("9.0.2", fixture!("9.0.2")),
    ];

    fn latest() -> &'static Fixture {
        &FIXTURES[FIXTURES.len() - 1].1
    }

    fn spawns(compact: &[u8]) -> Vec<SpawnExec> {
        let mut spawns = Vec::new();
        each_spawn(compact, |spawn| {
            spawns.push(spawn);
            Ok(())
        })
        .unwrap();
        spawns
    }

    /// The fixture's compact log converted to `format`, sorted as Bazel's own
    /// logs were, through the files `convert` reads and writes.
    fn converted(fixture: &Fixture, format: Format) -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let (compact, out) = (dir.path().join("compact"), dir.path().join("out"));
        std::fs::write(&compact, fixture.compact).unwrap();
        convert(&compact, &out, format, true).unwrap();
        std::fs::read(out).unwrap()
    }

    /// The binary log's spawns, their timings, which differ run to run,
    /// cleared.
    fn timeless(mut log: &[u8]) -> Vec<SpawnExec> {
        let mut spawns = Vec::new();
        while !log.is_empty() {
            let mut spawn = SpawnExec::decode_length_delimited(&mut log).unwrap();
            spawn.metrics = None;
            spawns.push(spawn);
        }
        spawns
    }

    /// The JSON log with each message's `"metrics"` block taken out.
    fn without_metrics(json: &str) -> String {
        const START: &str = ",\n  \"metrics\": {";
        const END: &str = "\n  }";
        let mut out = String::new();
        let mut rest = json;
        while let Some(start) = rest.find(START) {
            out.push_str(&rest[..start]);
            rest = &rest[start..];
            rest = &rest[rest.find(END).unwrap() + END.len()..];
        }
        out + rest
    }

    #[test]
    fn the_binary_log_is_bazels() {
        for (version, fixture) in &FIXTURES {
            let ours = timeless(&converted(fixture, Format::Binary));
            assert_eq!(ours.len(), 4, "Bazel {version}");
            assert_eq!(ours, timeless(fixture.binary), "Bazel {version}");
        }
    }

    #[test]
    fn the_json_log_is_bazels_byte_for_byte() {
        for (version, fixture) in &FIXTURES {
            let ours = String::from_utf8(converted(fixture, Format::Json)).unwrap();
            assert_eq!(
                without_metrics(&ours),
                without_metrics(fixture.json),
                "Bazel {version}"
            );
        }
    }

    #[test]
    fn metrics_print_as_bazel_prints_them() {
        let fixture = latest();
        let ours = json::spawn_exec(&spawns(fixture.compact)[0]);
        let block = &ours[ours.find("\"metrics\"").unwrap()..];
        for field in [
            "\"totalTime\": \"",
            "\"inputBytes\": \"0\"",
            "\"startTime\": \"20",
        ] {
            assert!(block.contains(field), "{field} in {block}");
        }
    }

    fn version(v: &str) -> Option<semver::Version> {
        Some(semver::Version::parse(v).unwrap())
    }

    fn asked(line: &[&str], v: Option<semver::Version>) -> Request {
        let flags: Vec<String> = line.iter().map(|s| s.to_string()).collect();
        request(
            &flags,
            "build",
            Some("/ws/pkg"),
            Some(Path::new("/ws")),
            v.as_ref(),
        )
    }

    fn files(files: &[(Format, &str)], sorted: bool, off: &[&str]) -> Request {
        Request::Files {
            files: files.iter().map(|(f, p)| (*f, PathBuf::from(p))).collect(),
            sorted,
            off: off.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn up_to_bazel_9_any_value_but_an_empty_one_is_a_path() {
        let v9 = || version("9.0.2");
        assert_eq!(
            asked(&["--execution_log_json_file=x.json"], v9()),
            files(
                &[(Format::Json, "/ws/pkg/x.json")],
                true,
                &["--execution_log_json_file="]
            )
        );
        assert_eq!(
            asked(&["--execution_log_binary_file=false"], v9()),
            files(
                &[(Format::Binary, "/ws/pkg/false")],
                true,
                &["--execution_log_binary_file="]
            )
        );
        assert_eq!(
            asked(
                &["--execution_log_json_file=x", "--execution_log_json_file="],
                v9()
            ),
            Request::None
        );
        assert_eq!(
            asked(&["--execution_log_compact_file=x"], v9()),
            Request::None
        );
    }

    #[test]
    fn from_bazel_10_a_boolean_is_not_a_path() {
        let v10 = || version("10.0.0-rc3");
        for line in [
            "--execution_log_json_file",
            "--execution_log_json_file=",
            "--execution_log_json_file=true",
        ] {
            assert_eq!(asked(&[line], v10()), Request::Bazels, "{line}");
        }
        for line in ["--execution_log_json_file=0", "--noexecution_log_json_file"] {
            assert_eq!(
                asked(&["--execution_log_json_file=x", line], v10()),
                Request::None,
                "{line}"
            );
        }
        assert_eq!(
            asked(&["--execution_log_binary_file=%workspace%/b.log"], v10()),
            files(
                &[(Format::Binary, "/ws/b.log")],
                true,
                &["--execution_log_binary_file=false"]
            )
        );
    }

    #[test]
    fn both_formats_and_rc_entries_are_read() {
        assert_eq!(
            asked(
                &[
                    "--default_override=0:build=--execution_log_binary_file=/tmp/b",
                    "--default_override=0:common=--execution_log_sort=false",
                    "--execution_log_json_file=/tmp/j",
                ],
                version("8.8.2"),
            ),
            files(
                &[(Format::Binary, "/tmp/b"), (Format::Json, "/tmp/j")],
                false,
                &["--execution_log_binary_file=", "--execution_log_json_file="],
            )
        );
    }

    #[test]
    fn a_bazel_without_the_compact_log_or_a_version_keeps_its_own() {
        for v in [version("7.3.2"), None] {
            assert_eq!(
                asked(&["--execution_log_json_file=x"], v.clone()),
                Request::Bazels,
                "{v:?}"
            );
            assert_eq!(asked(&[], v.clone()), Request::None, "{v:?}");
        }
    }

    #[test]
    fn an_unsorted_log_keeps_bazels_execution_order() {
        let fixture = latest();
        let dir = tempfile::tempdir().unwrap();
        let (compact, out) = (dir.path().join("compact"), dir.path().join("out"));
        std::fs::write(&compact, fixture.compact).unwrap();
        convert(&compact, &out, Format::Binary, false).unwrap();
        let mut theirs = spawns(fixture.compact);
        theirs.iter_mut().for_each(|s| s.metrics = None);
        assert_eq!(timeless(&std::fs::read(out).unwrap()), theirs);
    }
}
