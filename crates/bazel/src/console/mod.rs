//! Bazel's console, captured: the client's stderr read by this process and
//! forwarded on, rather than inherited, so it can be observed and rewritten on
//! the way.
//!
//! * [`capture`]: the fd the client gets as its stderr, a pipe or a PTY.
//! * [`forward`]: the thread that reads it, splits it into records and passes
//!   each through a [`forward::LineProcessor`] chain.
//! * [`processors`], [`filters`], [`report`], [`aggregate`]: the stages, described by
//!   [`Stage`], and [`line`], what they know of a record.
//! * [`log`]: the raw log a pipeline that changes the console keeps.
//! * [`render`]: curses output turned into plain lines.
//! * [`feed`]: what a hidden console would have shown, for a caller drawing
//!   its own screen.
//! * [`ansi`]: the escape sequences in a record.
//! * [`links`]: paths and URLs made clickable, for a terminal.
//!
//! A caller configures a [`Config`] once and builds a fresh [`Pipeline`] from
//! it per invocation, so one config serves every retry attempt.
//!
//! # Stage order
//!
//! Stages run in the order of their [`Phase`], and in the order given within
//! one, whatever order they were listed in: what an observer sees and what a
//! collapser folds then does not depend on which feature added its stage
//! first. A pipeline with a stage that hides lines ends with a
//! [`report::Summary`] of them.
//!
//! # Curses, and what is shown
//!
//! Bazel is always captured with its curses UI ([`Mode::Pty`]): it redraws
//! its progress area after every message, which ends each action's output,
//! so the stages can tell Bazel's messages from the output of the actions it
//! runs in any build. Plain lines (`--curses=no`) mark that end only with an
//! occasional progress line, which a short build may never print.
//!
//! What is shown is the [`Output`]. Where curses is shown (a terminal, or a
//! user who asked for it), the stages run on Bazel's records as it wrote
//! them and leave the rows curses redraws alone. Otherwise the forwarder
//! renders the console to plain lines first ([`render::Screen`]) and the
//! stages run on those, with no progress area to step around. Hidden
//! ([`Output::Hidden`]), it renders the same way but shows nothing: what
//! would be shown, and each progress area as Bazel erases it, go to the
//! pipeline's [`feed::Feed`].
//!
//! Inheriting stderr into a file or reading the BEP's progress events would
//! give a copy of the console. Neither sits between bazel and the terminal,
//! which rewriting a line before it is shown needs, and a file is not a TTY,
//! so bazel would drop its live progress UI. A PTY keeps that UI.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use regex::Regex;

pub mod aggregate;
pub mod ansi;
pub mod capture;
pub mod feed;
pub mod filters;
pub mod forward;
pub mod line;
pub mod links;
pub mod log;
pub mod processors;
pub mod render;
pub mod report;

use filters::{CollapseSimilar, DropLines, RelativizePaths, Rewrite};
use forward::LineProcessor;
use processors::{Aggregator, CollapseRepeats, HeldLine, Interceptor, LineMatcher, Observations};
use report::{RawLogState, RecapErrors, Stats, Style, Summary};

/// What the client writes to its stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Plain lines (`--curses=no`): progress comes as an occasional
    /// `[n / m]` line, the only end an action's output gets, so in a short
    /// build a message after action output may not be told from it.
    Pipe,
    /// Bazel's curses UI, through a pseudo-terminal or a pipe given
    /// `--curses=yes`: it redraws its progress area after every message,
    /// which ends each action's output. Records carry ANSI escapes; patterns
    /// match their text with escapes stripped. The capture always asks for
    /// it; [`Output`] decides what is forwarded.
    Pty,
}

/// What reaches the destination of a [`Mode::Pty`] console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// What Bazel wrote, curses UI and all: for a terminal, or a user who
    /// asked for curses.
    AsWritten,
    /// Plain lines ([`render::Screen`]): the progress area curses drew and
    /// erased is left out, and so is cursor movement; with `status_every`,
    /// Bazel's status line comes through at most that often.
    Lines { status_every: Option<Duration> },
    /// Nothing: for a caller that draws the screen itself. A [`Mode::Pty`]
    /// console is rendered to plain lines as for [`Output::Lines`], and what
    /// would be shown, with each progress area as Bazel erases it to redraw,
    /// goes to the pipeline's [`feed::Feed`] instead. The client's stdout,
    /// when it is a file of its own, is still shown: it is the program's
    /// output, not the console UI.
    Hidden,
}

impl Mode {
    /// [`Mode::Pty`] for a terminal destination, otherwise [`Mode::Pipe`].
    pub fn for_destination(is_terminal: bool) -> Mode {
        if is_terminal { Mode::Pty } else { Mode::Pipe }
    }
}

/// A pattern that does not compile. Displayed on one line: the regex
/// crate's own message draws the pattern with a caret under it.
#[derive(Debug, thiserror::Error)]
#[error("invalid regex for '{id}': {}", reason(source))]
pub struct PatternError {
    pub id: String,
    #[source]
    pub source: regex::Error,
}

/// The last line of a regex error, its reason: `unclosed group`.
fn reason(e: &regex::Error) -> String {
    let text = e.to_string();
    let last = text.lines().last().unwrap_or_default().trim();
    last.strip_prefix("error: ").unwrap_or(last).to_string()
}

/// Compile `pattern`, naming it `id` in the error.
pub fn compile(id: &str, pattern: &str) -> Result<Regex, PatternError> {
    Regex::new(pattern).map_err(|e| pattern_error(id, e))
}

fn pattern_error(id: &str, source: regex::Error) -> PatternError {
    PatternError {
        id: id.to_string(),
        source,
    }
}

fn compile_ids(
    patterns: impl IntoIterator<Item = (String, String)>,
) -> Result<Vec<(String, Regex)>, PatternError> {
    patterns
        .into_iter()
        .map(|(id, p)| compile(&id, &p).map(|r| (id, r)))
        .collect()
}

fn compile_all(patterns: impl IntoIterator<Item = String>) -> Result<Vec<Regex>, PatternError> {
    patterns.into_iter().map(|p| compile(&p, &p)).collect()
}

/// When a [`Stage`] runs, earliest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    /// Answer held lines, before anything else changes them.
    Intercept,
    /// Edit lines in place.
    Rewrite,
    /// See every line as it is shown, redacted (after the rewrites), before
    /// `drop` or a collapser hides any: a tip that quotes a line never shows
    /// what a rewrite masked. A line an intercept answer drops is gone.
    Observe,
    /// Hide lines by pattern.
    Drop,
    /// See what the stages before have left, for a report at the end. A line
    /// a collapser folds later is still seen.
    Report,
    /// Fold what is left into its first of a kind.
    Collapse,
    /// Fold runs of identical lines. Last, because its annotation is
    /// written with the record after a run, which a stage after it would
    /// read as part of that record.
    Repeat,
}

/// One configured stage of a console pipeline. Patterns are compiled by the
/// constructors.
#[derive(Debug, Clone)]
pub enum Stage {
    /// `(id, regex)` patterns recorded in [`Observations::matches`].
    Observe(Vec<(String, Regex)>),
    /// A pattern's matches aggregated in [`Observations::aggregates`].
    Aggregate(Box<aggregate::Spec>),
    /// Patterns that latch [`Observations::fatal`].
    Fatal(Vec<Regex>),
    /// `(id, regex)` patterns held for a [`processors::Verdict`], each for
    /// up to `timeout`.
    Intercept {
        patterns: Vec<(String, Regex)>,
        timeout: Duration,
    },
    /// [`filters::Rewrite`].
    Rewrite {
        regex: regex::bytes::Regex,
        replacement: String,
    },
    /// [`filters::RelativizePaths`] under the workspace `root`, relative to
    /// `cwd`.
    RelativizePaths {
        root: Option<String>,
        cwd: Option<String>,
    },
    /// [`filters::DropLines`].
    Drop(Vec<Regex>),
    /// [`report::RecapErrors`] of up to `max` errors, once `after` lines
    /// followed the first.
    RecapErrors { max: usize, after: usize },
    /// [`processors::CollapseRepeats`]; not where curses is shown, since its
    /// annotation, inserted before the record after a run, would land inside
    /// curses' redraw of its progress area: [`Config::stages`] leaves it out
    /// there.
    CollapseRepeats,
    /// [`filters::CollapseSimilar`].
    CollapseSimilar {
        severities: Vec<String>,
        groups: Vec<(String, Regex)>,
    },
}

impl Stage {
    pub fn observe(
        patterns: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Stage, PatternError> {
        Ok(Stage::Observe(compile_ids(patterns)?))
    }

    pub fn fatal(patterns: impl IntoIterator<Item = String>) -> Result<Stage, PatternError> {
        Ok(Stage::Fatal(compile_all(patterns)?))
    }

    pub fn intercept(
        patterns: impl IntoIterator<Item = (String, String)>,
        timeout: Duration,
    ) -> Result<Stage, PatternError> {
        Ok(Stage::Intercept {
            patterns: compile_ids(patterns)?,
            timeout,
        })
    }

    pub fn rewrite(pattern: &str, replacement: String) -> Result<Stage, PatternError> {
        let regex = regex::bytes::Regex::new(pattern).map_err(|e| pattern_error(pattern, e))?;
        Ok(Stage::Rewrite { regex, replacement })
    }

    pub fn drop(patterns: impl IntoIterator<Item = String>) -> Result<Stage, PatternError> {
        Ok(Stage::Drop(compile_all(patterns)?))
    }

    pub fn collapse_similar(
        severities: Vec<String>,
        groups: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Stage, PatternError> {
        Ok(Stage::CollapseSimilar {
            severities,
            groups: compile_ids(groups)?,
        })
    }

    /// The stage's name, as `bazel.output.<name>(...)` spells it.
    pub fn name(&self) -> &'static str {
        match self {
            Stage::Observe(_) => "observe",
            Stage::Aggregate(_) => "aggregate",
            Stage::Fatal(_) => "fatal",
            Stage::Intercept { .. } => "intercept",
            Stage::Rewrite { .. } => "rewrite",
            Stage::RelativizePaths { .. } => "relativize_paths",
            Stage::Drop(_) => "drop",
            Stage::RecapErrors { .. } => "recap_errors",
            Stage::CollapseRepeats => "collapse_repeats",
            Stage::CollapseSimilar { .. } => "collapse_similar",
        }
    }

    pub fn phase(&self) -> Phase {
        match self {
            Stage::Observe(_) | Stage::Aggregate(_) | Stage::Fatal(_) => Phase::Observe,
            Stage::Intercept { .. } => Phase::Intercept,
            Stage::Rewrite { .. } | Stage::RelativizePaths { .. } => Phase::Rewrite,
            Stage::Drop(_) => Phase::Drop,
            Stage::RecapErrors { .. } => Phase::Report,
            Stage::CollapseSimilar { .. } => Phase::Collapse,
            Stage::CollapseRepeats => Phase::Repeat,
        }
    }

    /// Whether the stage can change what is shown, so the console it was
    /// given needs keeping in a raw log.
    pub fn alters(&self) -> bool {
        !matches!(
            self,
            Stage::Observe(_) | Stage::Aggregate(_) | Stage::Fatal(_) | Stage::RecapErrors { .. }
        )
    }

    /// Whether the stage hides lines by itself, which the [`Summary`]
    /// counts. A held line a consumer drops is its decision, not counted.
    fn hides(&self) -> bool {
        matches!(self, Stage::Drop(_) | Stage::CollapseSimilar { .. })
    }
}

/// What a captured invocation does with its console.
#[derive(Debug, Clone)]
pub struct Config {
    pub mode: Mode,
    /// In [`Phase`] order.
    stages: Vec<Stage>,
    /// Where a pipeline that [`Stage::alters`] the console keeps its raw log
    /// ([`Config::keeps_log`]); None keeps none.
    pub log: Option<log::Place>,
    /// How report lines are set apart from bazel's.
    pub style: Style,
    /// Capture through a pseudo-terminal rather than a pipe: for a terminal
    /// destination that keeps the curses UI, whose size Bazel then follows.
    pub pty: bool,
    /// What is forwarded; see [`Output`].
    pub output: Output,
    /// Forward no colour: for a destination that renders none, or a user
    /// who asked for none.
    pub strip_colour: bool,
    /// The user's own `--color`, as yes or no, when they gave one: what a
    /// separate stdout shows, as `strip_colour` is for stderr.
    pub user_colour: Option<bool>,
    /// Ask bazel for colour (`--color=yes`), by which the stages tell its
    /// messages from the output of the actions it runs ([`line`]). What is
    /// shown keeps it only as [`Config::strip_colour`] says.
    pub colour: bool,
    /// For a terminal: write the paths and URLs Bazel's lines and the
    /// recap name, and the paths the summary names, as OSC 8 hyperlinks
    /// ([`links`]), a relative path resolved from the first of these
    /// directories it exists under. A hidden console ([`Output::Hidden`])
    /// links nothing.
    pub links: Option<Vec<std::path::PathBuf>>,
}

impl Config {
    /// `stages` are put in [`Phase`] order, keeping the given order within a
    /// phase.
    pub fn new(mode: Mode, mut stages: Vec<Stage>, style: Style) -> Self {
        stages.sort_by_key(Stage::phase);
        Config {
            mode,
            stages,
            log: None,
            style,
            pty: mode == Mode::Pty,
            output: Output::AsWritten,
            strip_colour: false,
            user_colour: None,
            links: None,
            colour: false,
        }
    }

    /// This config with its raw log kept at `place`.
    pub fn with_log(mut self, place: log::Place) -> Self {
        self.log = Some(place);
        self
    }

    /// This config linking a relative path from `root`, the workspace a
    /// build runs in, before the directory the task runs from (the last of
    /// [`Config::links`]), each directory tried once.
    pub fn link_from(&mut self, root: std::path::PathBuf) {
        if let Some(bases) = &mut self.links {
            let cwd = bases.pop().filter(|cwd| *cwd != root);
            *bases = std::iter::once(root).chain(cwd).collect();
        }
    }

    /// A [`links::Linker`] for what is shown, when [`Config::links`] asks for
    /// one and something is shown.
    fn linker(&self) -> Option<links::Linker> {
        let bases = self.links.clone()?;
        (self.output != Output::Hidden).then(|| links::Linker::new(bases))
    }

    /// Whether the pipeline keeps a raw log, given a [`Config::log`]: when a
    /// stage can change what is shown, or nothing is ([`Output::Hidden`]).
    pub fn keeps_log(&self) -> bool {
        self.output == Output::Hidden || self.stages().iter().any(|s| s.alters())
    }

    /// The bazel flags the capture needs, to go after the user's: whether it
    /// writes to a terminal, curses for [`Mode::Pty`], `--color=yes` with
    /// [`Config::colour`], and `--show_progress`.
    pub fn bazel_flags(&self) -> Vec<&'static str> {
        let mut flags = match (self.mode, self.pty) {
            (Mode::Pipe, _) => vec!["--isatty=0", "--curses=no"],
            (Mode::Pty, true) => vec!["--isatty=1", "--curses=yes"],
            (Mode::Pty, false) => vec!["--isatty=0", "--curses=yes"],
        };
        if self.colour {
            flags.push("--color=yes");
        }
        // Its redraws are what end an action's output: without progress
        // (an rc's `--noshow_progress`) nothing would, and every message
        // after the first action's output would read as output too. The
        // user's flag still decides whether plain lines show it.
        flags.push("--show_progress");
        flags
    }

    /// What the forwarder does past the stages, for
    /// [`forward::Forwarder::spawn_logged`].
    pub fn shape(&self) -> forward::Shape {
        forward::Shape {
            strip_colour: self.strip_colour,
            lines: match (self.mode, self.output) {
                (Mode::Pty, Output::Lines { status_every }) => Some(status_every),
                (Mode::Pty, Output::Hidden) => Some(None),
                _ => None,
            },
            feed: None,
            hidden: self.output == Output::Hidden,
        }
    }

    /// Whether what is forwarded is Bazel's curses UI: the stages then run on
    /// its records as it wrote them, and leave the rows curses redraws as
    /// they are. Otherwise they run on the plain lines the rendering settles.
    pub fn shows_curses(&self) -> bool {
        self.mode == Mode::Pty && self.output == Output::AsWritten
    }

    /// The stages a pipeline runs: all, but [`Stage::CollapseRepeats`] where
    /// [`Config::shows_curses`], whose annotation would land inside curses'
    /// redraw of its progress area.
    pub fn stages(&self) -> Vec<&Stage> {
        self.stages
            .iter()
            .filter(|s| !(self.shows_curses() && matches!(s, Stage::CollapseRepeats)))
            .collect()
    }

    /// Whether a build holds lines for hooks to answer: it has an
    /// `intercept` stage.
    pub fn intercepts(&self) -> bool {
        self.stages
            .iter()
            .any(|s| matches!(s, Stage::Intercept { .. }))
    }

    /// A fresh chain for one invocation, with its own observations, event
    /// channel and raw log.
    pub fn pipeline(&self) -> Pipeline {
        let pty = self.shows_curses();
        let observations = Arc::new(Observations::default());
        let stats = Arc::new(Stats::default());
        let (events_tx, events) = if self.intercepts() {
            let (tx, rx) = mpsc::channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let log = match &self.log {
            Some(place) if self.keeps_log() => log::RawLog::create(place)
                .inspect_err(|e| tracing::debug!("no raw console log at {place}: {e}"))
                .ok(),
            _ => None,
        };
        let mut chain: Vec<Box<dyn LineProcessor>> = vec![];
        // What `relativize_paths` says each path it made relative names, for
        // the links, when there are any.
        let hints = self.linker().map(|_| links::Hints::default());
        for stage in self.stages() {
            let stage: Box<dyn LineProcessor> = match stage.clone() {
                Stage::Observe(patterns) => {
                    let o = observations.clone();
                    Box::new(LineMatcher::new(
                        patterns,
                        Box::new(move |id, line| o.record_match(id, line)),
                    ))
                }
                Stage::Aggregate(spec) => {
                    let slot = observations.add_tally(*spec);
                    Box::new(Aggregator::new(observations.clone(), slot))
                }
                Stage::Fatal(patterns) => {
                    let o = observations.clone();
                    let patterns = patterns
                        .into_iter()
                        .map(|r| (r.as_str().to_string(), r))
                        .collect();
                    Box::new(LineMatcher::new(
                        patterns,
                        Box::new(move |_, line| o.set_fatal(line)),
                    ))
                }
                Stage::Intercept { patterns, timeout } => Box::new(
                    Interceptor::new(
                        patterns,
                        events_tx.clone().expect("an intercept stage has a channel"),
                        timeout,
                    )
                    .under_pty(pty),
                ),
                Stage::Rewrite { regex, replacement } => {
                    Box::new(Rewrite::new(regex, replacement).under_pty(pty))
                }
                Stage::RelativizePaths { root, cwd } => Box::new(
                    RelativizePaths::new(root, cwd)
                        .under_pty(pty)
                        .hinting(hints.clone()),
                ),
                Stage::Drop(patterns) => Box::new(DropLines::new(patterns, pty, stats.clone())),
                Stage::RecapErrors { max, after } => Box::new(
                    RecapErrors::new(max, after, self.style.clone())
                        .linking(self.linker().zip(hints.clone())),
                ),
                Stage::CollapseRepeats => Box::<CollapseRepeats>::default(),
                Stage::CollapseSimilar { severities, groups } => {
                    Box::new(CollapseSimilar::new(severities, groups, pty, stats.clone()))
                }
            };
            chain.push(stage);
        }
        if let Some(linker) = self.linker() {
            chain.push(Box::new(links::Links {
                linker,
                hints: hints.clone().unwrap_or_default(),
            }));
        }
        if let Some(l) = &log {
            stats.keep_groups(l.path());
        }
        let alters = self.keeps_log();
        let state = match (&log, &self.log) {
            (Some(l), _) => RawLogState::Kept(l.path().to_path_buf(), l.cut()),
            (None, Some(_)) if alters => RawLogState::Failed,
            _ => RawLogState::Off,
        };
        if self.stages().iter().any(|s| s.hides()) || !matches!(state, RawLogState::Off) {
            chain.push(Box::new(Summary::new(
                stats,
                state,
                self.style.clone(),
                self.links.is_some() && self.output != Output::Hidden,
            )));
        }
        let feed = (self.output == Output::Hidden).then(|| Arc::new(feed::Feed::default()));
        let mut shape = self.shape();
        shape.feed = feed.clone();
        Pipeline {
            chain,
            observations,
            events,
            log,
            shape,
            feed,
        }
    }
}

/// One invocation's chain and what it reports through.
pub struct Pipeline {
    pub chain: Vec<Box<dyn LineProcessor>>,
    pub observations: Arc<Observations>,
    /// The held lines, when the config has an intercept stage.
    pub events: Option<mpsc::Receiver<HeldLine>>,
    /// The raw log, for the forwarder to write.
    pub log: Option<log::RawLog>,
    /// [`Config::shape`], for the forwarder.
    pub shape: forward::Shape,
    /// What is shown, for a caller drawing its own screen: with
    /// [`Output::Hidden`].
    pub feed: Option<Arc<feed::Feed>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::forward::Forwarder;
    use crate::console::processors::Verdict;
    use crate::testing::SharedSink;

    /// `input` as bazel writes it, its messages coloured.
    fn coloured(input: &[u8]) -> Vec<u8> {
        let text = String::from_utf8_lossy(input);
        text.split_inclusive('\n')
            .map(|l| match l.strip_suffix('\n') {
                Some(l) => crate::console::line::tests::bazel(l) + "\n",
                None => crate::console::line::tests::bazel(l),
            })
            .collect::<String>()
            .into_bytes()
    }

    /// Run `input`, its messages coloured as bazel colours them, through
    /// `config`'s pipeline; what came out, uncoloured.
    fn run(config: &Config, input: &[u8]) -> (String, Pipeline) {
        let input = coloured(input);
        let mut pipeline = config.pipeline();
        let chain = std::mem::take(&mut pipeline.chain);
        let consumer = pipeline.events.take().map(|events| {
            std::thread::spawn(move || {
                for ev in events {
                    let _ = ev
                        .reply
                        .send(Verdict::Replace(b"(password elided)".to_vec()));
                }
            })
        });
        let sink = SharedSink::default();
        let mut forwarder = Forwarder::spawn_logged(
            Box::new(std::io::Cursor::new(input.to_vec())),
            Box::new(sink.clone()),
            chain,
            pipeline.log.take(),
            pipeline.shape.clone(),
        )
        .unwrap();
        forwarder.join().unwrap();
        if let Some(c) = consumer {
            c.join().unwrap();
        }
        (
            crate::console::line::tests::unbazel(&sink.bytes()),
            pipeline,
        )
    }

    /// Listed out of order: the interceptor answers first, observers see
    /// every line it keeps (repeats included), and the collapser folds
    /// last.
    #[test]
    fn stages_run_in_phase_order_whatever_order_they_are_listed_in() {
        let config = Config::new(
            Mode::Pipe,
            vec![
                Stage::CollapseRepeats,
                Stage::intercept(
                    [("secret".into(), "password".into())],
                    Duration::from_secs(30),
                )
                .unwrap(),
                Stage::observe([("warn".into(), "^WARNING:".into())]).unwrap(),
                Stage::fatal(["Server terminated abruptly".into()]).unwrap(),
            ],
            Style::default(),
        );
        let names: Vec<_> = config.stages().iter().map(|s| s.name()).collect();
        assert_eq!(names, ["intercept", "observe", "fatal", "collapse_repeats"]);

        let input = b"WARNING: flaky\nWARNING: flaky\nWARNING: flaky\npassword=hunter2\n\
                      password=hunter2\nServer terminated abruptly\n";
        let (out, pipeline) = run(&config, input);
        assert_eq!(
            out,
            "WARNING: flaky\n(last line repeated 2 more times)\n(password elided)\n\
             (last line repeated 1 more time)\nServer terminated abruptly\n"
        );
        let hits = pipeline.observations.matches();
        assert_eq!(hits.len(), 3);
        assert!(hits.iter().all(|(id, _)| id == "warn"));
        assert_eq!(
            pipeline.observations.fatal_line().as_deref(),
            Some("Server terminated abruptly")
        );
    }

    /// The defaults a build gets, end to end: paths shortened, noise
    /// dropped, warnings collapsed, the summary naming a log of the raw
    /// console.
    #[test]
    fn a_hiding_pipeline_summarizes_and_keeps_the_raw_console() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(
            Mode::Pipe,
            vec![
                Stage::collapse_similar(vec!["WARNING".into()], []).unwrap(),
                Stage::drop(["^DEBUG: Rule".into()]).unwrap(),
                Stage::RelativizePaths {
                    root: Some("/ws".into()),
                    cwd: None,
                },
            ],
            Style::default(),
        )
        .with_log(log::Place::In(dir.path().to_path_buf()));
        let input = b"WARNING: /ws/a/BUILD:1:1: target '//a:a' is deprecated\n\
                      DEBUG: Rule 'x' indicated that a canonical reproducible form\n\
                      WARNING: /ws/b/BUILD:1:1: target '//b:b' is deprecated\n\
                      INFO: Build completed successfully\n";
        let (out, _) = run(&config, input);
        let mut files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        assert_eq!(
            files.len(),
            2,
            "the log and its one collapsed kind: {files:?}"
        );
        let (group, log) = (&files[0], &files[1]);
        assert_eq!(group, &log::RawLog::sibling(log, "collapsed-1"));
        assert_eq!(std::fs::read(log).unwrap(), coloured(input));
        assert_eq!(
            std::fs::read_to_string(group).unwrap(),
            "WARNING: a/BUILD:1:1: target '//a:a' is deprecated\n\
             WARNING: b/BUILD:1:1: target '//b:b' is deprecated\n",
            "as shown, relativized"
        );
        assert_eq!(
            out,
            format!(
                "WARNING: a/BUILD:1:1: target '//a:a' is deprecated\n\
                 INFO: Build completed successfully\n\
                 INFO: Aspect hid 2 console lines; the raw console is in {}\n  \
                 1 more like: WARNING: a/BUILD:1:1: target '//a:a' is deprecated\n    \
                 all 2 in {}\n",
                log.display(),
                group.display()
            )
        );
    }

    /// The repeat annotation rides on the record after a run, so nothing
    /// may run after it and read the two as one.
    #[test]
    fn repeats_fold_after_similar_lines_collapse() {
        let config = Config::new(
            Mode::Pipe,
            vec![
                Stage::CollapseRepeats,
                Stage::collapse_similar(vec!["WARNING".into()], []).unwrap(),
            ],
            Style::default(),
        );
        let (out, _) = run(
            &config,
            b"WARNING: a 1\nWARNING: a 2\nWARNING: a 2\nINFO: next\nERROR: boom\nx\nx\n",
        );
        assert_eq!(
            out,
            "WARNING: a 1\nINFO: next\nERROR: boom\nx\n(last line repeated 1 more time)\n\
             INFO: Aspect hid 2 console lines\n  2 more like: WARNING: a 1\n"
        );
    }

    /// The recap quotes an error as the user saw it: redacted, shortened.
    #[test]
    fn the_recap_quotes_rewritten_errors() {
        let config = Config::new(
            Mode::Pipe,
            vec![
                Stage::RecapErrors {
                    max: 5,
                    after: report::RECAP_AFTER_RECORDS,
                },
                Stage::rewrite("token=[A-Za-z0-9]+", "token=***".into()).unwrap(),
            ],
            Style::default(),
        );
        let mut input = b"ERROR: fetch failed: https://x.dev?token=Secret123\n".to_vec();
        for _ in 0..report::RECAP_AFTER_RECORDS {
            input.extend_from_slice(b"noise\n");
        }
        let (out, _) = run(&config, &input);
        assert!(!out.contains("Secret123"), "{out}");
        assert_eq!(out.matches("token=***").count(), 2, "{out}");
    }

    #[test]
    fn a_pipeline_that_only_observes_keeps_no_log() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(
            Mode::Pipe,
            vec![Stage::observe([("w".into(), "W".into())]).unwrap()],
            Style::default(),
        )
        .with_log(log::Place::In(dir.path().to_path_buf()));
        assert!(config.pipeline().log.is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_pty_capture_never_collapses_repeats() {
        let dir = tempfile::tempdir().unwrap();
        let config = |mode| {
            Config::new(mode, vec![Stage::CollapseRepeats], Style::default())
                .with_log(log::Place::In(dir.path().to_path_buf()))
        };
        assert_eq!(
            config(Mode::Pipe).pipeline().chain.len(),
            2,
            "the stage and the summary naming its log"
        );
        let pty = config(Mode::Pty);
        assert!(pty.stages().is_empty());
        assert!(
            pty.pipeline().log.is_none(),
            "nothing is altered, so no log"
        );
    }

    #[test]
    fn an_empty_config_builds_an_empty_chain() {
        let pipeline = Config::new(Mode::Pty, vec![], Style::default()).pipeline();
        assert!(pipeline.chain.is_empty());
        assert!(pipeline.events.is_none());
        assert!(pipeline.log.is_none());
    }

    #[test]
    fn an_invalid_pattern_names_its_id() {
        let err = Stage::observe([("broken".into(), "(".into())]).unwrap_err();
        assert_eq!(err.id, "broken");
        assert_eq!(
            err.to_string(),
            "invalid regex for 'broken': unclosed group"
        );
        let err = Stage::drop(["(".into()]).unwrap_err();
        assert!(err.to_string().starts_with("invalid regex for '(':"));
    }

    #[test]
    fn each_mode_has_the_flags_that_fit_it() {
        assert_eq!(Mode::for_destination(true), Mode::Pty);
        assert_eq!(Mode::for_destination(false), Mode::Pipe);
        let mut config = Config::new(Mode::Pipe, vec![], Style::default());
        config.colour = true;
        assert_eq!(
            config.bazel_flags(),
            [
                "--isatty=0",
                "--curses=no",
                "--color=yes",
                "--show_progress"
            ]
        );
        let mut config = Config::new(Mode::Pty, vec![], Style::default());
        assert_eq!(
            config.bazel_flags(),
            ["--isatty=1", "--curses=yes", "--show_progress"]
        );
        config.pty = false;
        assert_eq!(
            config.bazel_flags(),
            ["--isatty=0", "--curses=yes", "--show_progress"]
        );
    }

    /// Real bazel 9 consoles, recorded under a PTY and through a pipe with a
    /// genrule tool and tests that print every kind of line a stage might
    /// mistake for bazel's: header-shaped, coloured exactly as bazel colours
    /// (as a nested bazel would), progress-shaped, indented, blank.
    const RECORDED: &[(&str, Mode, &[u8])] = &[
        (
            "pipe_genrules",
            Mode::Pipe,
            include_bytes!("testdata/pipe_genrules.txt"),
        ),
        (
            "pipe_no_color",
            Mode::Pipe,
            include_bytes!("testdata/pipe_no_color.txt"),
        ),
        (
            "pipe_test_all",
            Mode::Pipe,
            include_bytes!("testdata/pipe_test_all.txt"),
        ),
        (
            "pipe_test_cached",
            Mode::Pipe,
            include_bytes!("testdata/pipe_test_cached.txt"),
        ),
        (
            "pipe_test_streamed_progress",
            Mode::Pipe,
            include_bytes!("testdata/pipe_test_streamed_progress.txt"),
        ),
        (
            "pipe_canonical_7",
            Mode::Pipe,
            include_bytes!("testdata/pipe_canonical_7.txt"),
        ),
        (
            "pipe_canonical_8",
            Mode::Pipe,
            include_bytes!("testdata/pipe_canonical_8.txt"),
        ),
        (
            "pipe_canonical_9",
            Mode::Pipe,
            include_bytes!("testdata/pipe_canonical_9.txt"),
        ),
        (
            "pipe_keep_going",
            Mode::Pipe,
            include_bytes!("testdata/pipe_keep_going.txt"),
        ),
        (
            "pty_keep_going",
            Mode::Pty,
            include_bytes!("testdata/pty_keep_going.txt"),
        ),
        (
            "pty_tool_redraw",
            Mode::Pty,
            include_bytes!("testdata/pty_tool_redraw.txt"),
        ),
        (
            "pipe_tool_tail",
            Mode::Pipe,
            include_bytes!("testdata/pipe_tool_tail.txt"),
        ),
        (
            "pipe_analysis_errors",
            Mode::Pipe,
            include_bytes!("testdata/pipe_analysis_errors.txt"),
        ),
        (
            "pty_tool_tail",
            Mode::Pty,
            include_bytes!("testdata/pty_tool_tail.txt"),
        ),
        (
            "pty_bazel7",
            Mode::Pty,
            include_bytes!("testdata/pty_bazel7.txt"),
        ),
        (
            "pty_bazel8",
            Mode::Pty,
            include_bytes!("testdata/pty_bazel8.txt"),
        ),
        (
            "pty_bazel8_mixed",
            Mode::Pty,
            include_bytes!("testdata/pty_bazel8_mixed.txt"),
        ),
        (
            "pty_narrow_bazel7",
            Mode::Pty,
            include_bytes!("testdata/pty_narrow_bazel7.txt"),
        ),
        (
            "pty_narrow_bazel8",
            Mode::Pty,
            include_bytes!("testdata/pty_narrow_bazel8.txt"),
        ),
        (
            "pty_bes_narrow",
            Mode::Pty,
            include_bytes!("testdata/pty_bes_narrow.txt"),
        ),
        (
            "pipe_action_error_text",
            Mode::Pipe,
            include_bytes!("testdata/pipe_action_error_text.txt"),
        ),
        (
            "pty_action_error_text",
            Mode::Pty,
            include_bytes!("testdata/pty_action_error_text.txt"),
        ),
        (
            "pty_test_cached",
            Mode::Pty,
            include_bytes!("testdata/pty_test_cached.txt"),
        ),
        (
            "pipe_test_errors",
            Mode::Pipe,
            include_bytes!("testdata/pipe_test_errors.txt"),
        ),
        (
            "pipe_test_streamed",
            Mode::Pipe,
            include_bytes!("testdata/pipe_test_streamed.txt"),
        ),
        (
            "pipe_timestamps",
            Mode::Pipe,
            include_bytes!("testdata/pipe_timestamps.txt"),
        ),
        (
            "pty_curses_no",
            Mode::Pty,
            include_bytes!("testdata/pty_curses_no.txt"),
        ),
        (
            "pty_genrules",
            Mode::Pty,
            include_bytes!("testdata/pty_genrules.txt"),
        ),
        (
            "pty_noshow_progress",
            Mode::Pty,
            include_bytes!("testdata/pty_noshow_progress.txt"),
        ),
        (
            "pty_test_all",
            Mode::Pty,
            include_bytes!("testdata/pty_test_all.txt"),
        ),
        (
            "pty_test_streamed",
            Mode::Pty,
            include_bytes!("testdata/pty_test_streamed.txt"),
        ),
        (
            "pty_timestamps",
            Mode::Pty,
            include_bytes!("testdata/pty_timestamps.txt"),
        ),
    ];

    /// What the recorded tool and tests print: every one must survive.
    const OUTPUT: &[&str] = &[
        "tool line A",
        "tool plain warning",
        "tool coloured warning",
        "tool coloured error",
        "model weights",
        "indented tool line",
        "coloured progress-shaped",
        "indented after coloured loading",
        "fake progress",
        "indented after fake progress",
        "Rule 'x' indicated",
        "indented after coloured debug",
        "stdout line one",
        "stdout coloured warning",
        "tool tail",
        "test out",
        "from test",
        "indented test",
        "from failing test",
        "in test",
        "indented after wait",
        "nested bazel progress",
        "step 1",
        "step 2",
        "tool same",
        "unused import \"os\"",
        "unused import \"sys\"",
        "b.py:7 import sys",
        "tool partial warning 1",
        "tool partial warning 2",
        "(tool partial)",
        "nested loading",
    ];

    /// The `Console` feature's defaults, for `agent` or not.
    fn defaults(mode: Mode, agent: bool) -> Config {
        let mut stages = vec![Stage::RelativizePaths {
            root: Some("/ws".into()),
            cwd: None,
        }];
        // Mirrors `AGENT_DROP` in bazel/console.axl.
        if agent {
            stages.push(
                Stage::drop([
                    r"^\[[\d,]+ / [\d,]+\] ".to_string(),
                    r"^(Loading|Analyzing|Computing main repo mapping):".to_string(),
                ])
                .unwrap(),
            );
        }
        stages.extend([
            Stage::collapse_similar(vec!["WARNING".into()], []).unwrap(),
            Stage::RecapErrors {
                max: report::RECAP_MAX,
                after: report::RECAP_AFTER_RECORDS,
            },
        ]);
        Config::new(mode, stages, Style::default())
    }

    /// In a pipe the blank line right after a dropped progress line goes
    /// with it; one after a record an earlier stage dropped in between stays.
    #[test]
    fn a_blank_after_a_record_dropped_upstream_stays() {
        struct DropX;
        impl LineProcessor for DropX {
            fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
                (!ansi::text(record).starts_with("X ")).then(|| record.to_vec())
            }
        }
        let stats = Arc::new(Stats::default());
        let chain: Vec<Box<dyn LineProcessor>> = vec![
            Box::new(DropX),
            Box::new(filters::DropLines::new(
                vec![regex::Regex::new("^Analyzing:").unwrap()],
                false,
                stats,
            )),
        ];
        let sink = SharedSink::default();
        let input =
            b"\x1b[32mAnalyzing:\x1b[0m 2 targets\nX dropped by a hook\n\nINFO: after\n".to_vec();
        let mut forwarder = Forwarder::spawn(
            Box::new(std::io::Cursor::new(input)),
            Box::new(sink.clone()),
            chain,
        )
        .unwrap();
        forwarder.join().unwrap();
        assert_eq!(ansi::text(&sink.bytes()), "\nINFO: after\n");
    }

    /// A line a stage early in the chain drops (a `intercept` hook's `drop()`)
    /// still ends a later stage's open body, so the lines under it are not
    /// taken for the body of a message collapsed before it.
    #[test]
    fn a_line_dropped_upstream_ends_a_later_stages_body() {
        struct DropSecret;
        impl LineProcessor for DropSecret {
            fn process(&mut self, record: &[u8]) -> Option<Vec<u8>> {
                let text = ansi::text(record);
                (!text.contains("secret")).then(|| record.to_vec())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("console-1-2-3.log");
        let stats = Arc::new(Stats::default());
        stats.keep_groups(&log);
        let chain: Vec<Box<dyn LineProcessor>> = vec![
            Box::new(DropSecret),
            Box::new(filters::CollapseSimilar::new(
                vec!["WARNING".into()],
                vec![],
                false,
                stats.clone(),
            )),
        ];
        let sink = SharedSink::default();
        let input = coloured(
            b"WARNING: same 1\nWARNING: same 2\n  body of 2\nDEBUG: secret header\n  \
              line under the dropped one\nDEBUG: after\n",
        );
        let mut forwarder = Forwarder::spawn(
            Box::new(std::io::Cursor::new(input)),
            Box::new(sink.clone()),
            chain,
        )
        .unwrap();
        forwarder.join().unwrap();
        let out = crate::console::line::tests::unbazel(&sink.bytes());
        assert!(out.contains("line under the dropped one"), "{out}");
        assert_eq!(stats.hidden(), 2, "the collapsed warning and its own body");
        // The kind's file is written once it hides one and finished by the
        // summary, which this chain has none of: finished here.
        stats.finish_groups();
        assert_eq!(
            std::fs::read_to_string(log::RawLog::sibling(&log, "collapsed-1")).unwrap(),
            "WARNING: same 1\nWARNING: same 2\n  body of 2\n",
            "the line under the dropped one is not the warning's"
        );
    }

    /// A redaction hook dropping every `Fetching` line of a real PTY console
    /// changes nothing on screen it should not: the progress area keeps its
    /// lines, and no output is lost.
    #[test]
    fn an_intercept_stage_leaves_the_pty_progress_area_alone() {
        let input: &[u8] = include_bytes!("testdata/pty_bazel8.txt");
        let config = Config::new(
            Mode::Pty,
            vec![
                Stage::intercept(
                    [("f".into(), "Fetching|Analyzing:|stdout line".into())],
                    Duration::from_secs(30),
                )
                .unwrap(),
            ],
            Style::default(),
        );
        let mut pipeline = config.pipeline();
        let events = pipeline.events.take().unwrap();
        let consumer = std::thread::spawn(move || {
            for ev in events {
                let _ = ev.reply.send(Verdict::Drop);
            }
        });
        let sink = SharedSink::default();
        let mut forwarder = Forwarder::spawn(
            Box::new(std::io::Cursor::new(input.to_vec())),
            Box::new(sink.clone()),
            std::mem::take(&mut pipeline.chain),
        )
        .unwrap();
        forwarder.join().unwrap();
        drop(pipeline);
        consumer.join().unwrap();
        let (before, after) = (ansi::text(input), ansi::text(&sink.bytes()));
        let fetching = |t: &str| {
            t.lines()
                .filter(|l| l.trim_start().starts_with("Fetching"))
                .count()
        };
        assert_eq!(
            fetching(&after),
            fetching(&before),
            "progress lines kept:\n{after}"
        );
        // Skymeld's phase line, drawn inside action output, too.
        assert_eq!(
            after.matches("Analyzing:").count(),
            before.matches("Analyzing:").count(),
            "phase lines kept:\n{after}"
        );
        assert!(
            after.matches("stdout line one").count() < before.matches("stdout line one").count(),
            "a message line can still be dropped:\n{after}"
        );
    }

    /// One macro's `print()` twice, the same first line over different
    /// values: both, and every line under them, show.
    #[test]
    fn a_print_is_never_collapsed() {
        let (out, _) = run(
            &defaults(Mode::Pipe, false),
            b"DEBUG: defs.bzl:2:10: deps:\n  //x:one\n  //x:two\n\
              DEBUG: defs.bzl:2:10: deps:\n  //y:three\n  //y:four\n",
        );
        for line in ["//x:one", "//x:two", "//y:three", "//y:four"] {
            assert!(out.contains(line), "{line} lost:\n{out}");
        }
        assert_eq!(
            out.matches("DEBUG: defs.bzl:2:10: deps:").count(),
            2,
            "{out}"
        );
    }

    /// Every row curses erases to redraw is one the classifier calls
    /// [`line::Line::drawn`] (or blank): the rows no stage may change the
    /// height of under a PTY.
    #[test]
    fn every_row_curses_erases_is_known_as_drawn() {
        for &(name, mode, input) in RECORDED {
            if mode != Mode::Pty {
                continue;
            }
            let mut records: Vec<(&[u8], bool)> = vec![];
            let (mut start, mut i) = (0, 0);
            while i < input.len() {
                let crlf = input[i] == b'\r' && input.get(i + 1) == Some(&b'\n');
                if crlf || input[i] == b'\r' || input[i] == b'\n' {
                    records.push((&input[start..i], crlf || input[i] == b'\n'));
                    i += if crlf { 2 } else { 1 };
                    start = i;
                } else {
                    i += 1;
                }
            }
            let mut classifier = line::Classifier::default();
            let mut rows: Vec<line::Line> = vec![];
            for (record, ends_row) in records {
                let up = record.windows(4).filter(|w| *w == b"\x1b[1A").count();
                let line = classifier.classify(record);
                if up > 0 {
                    let keep = rows.len().saturating_sub(up);
                    for row in &rows[keep..] {
                        // The one tool in the recordings that redraws its own
                        // line, with the same bytes curses uses.
                        let tools_own = name == "pty_tool_redraw" && row.text == "step 1";
                        assert!(
                            tools_own || row.drawn || row.kind == line::Kind::Blank,
                            "{name}: curses erased {:?}, not known as drawn",
                            row.text
                        );
                    }
                    rows.truncate(keep);
                }
                if ends_row {
                    rows.push(line);
                }
            }
        }
    }

    /// A `drop` of the canonical-form advice takes the call stack Bazel 7/8
    /// print under it, and Bazel 9's `defined by rule` line, from the real
    /// consoles.
    #[test]
    fn a_drop_takes_the_canonical_form_advice_with_its_stack() {
        for &(name, mode, input) in RECORDED {
            if !(name.starts_with("pipe_canonical") || name == "pty_bazel8_mixed") {
                continue;
            }
            let drop = Stage::drop([
                r"^DEBUG: (?:Rule|Repo) '[^']*' indicated that a canonical reproducible form can be obtained"
                    .to_string(),
                r"^DEBUG: Repo(?:sitory)? \S+ (?:instantiated at:|defined by rule )".to_string(),
            ])
            .unwrap();
            let mut pipeline = Config::new(mode, vec![drop], Style::default()).pipeline();
            let sink = SharedSink::default();
            Forwarder::spawn(
                Box::new(std::io::Cursor::new(input.to_vec())),
                Box::new(sink.clone()),
                std::mem::take(&mut pipeline.chain),
            )
            .unwrap()
            .join()
            .unwrap();
            let after = ansi::text(&sink.bytes());
            let shown = &after[..after.find("INFO: Aspect hid").unwrap_or(after.len())];
            for noise in [
                "indicated that a canonical",
                "instantiated at:",
                "defined by rule",
                "Repository rule",
                "<builtin>",
                "http.bzl:4",
            ] {
                assert!(
                    !shown.contains(noise),
                    "{name}: {noise:?} is left:\n{after}"
                );
            }
        }
    }

    /// `input` through a pipeline of no stages that renders plain lines.
    fn rendered(input: &[u8]) -> String {
        let mut config = Config::new(Mode::Pty, vec![], Style::default());
        config.output = Output::Lines { status_every: None };
        config.pty = false;
        let mut pipeline = config.pipeline();
        let sink = SharedSink::default();
        Forwarder::spawn_logged(
            Box::new(std::io::Cursor::new(input.to_vec())),
            Box::new(sink.clone()),
            std::mem::take(&mut pipeline.chain),
            None,
            pipeline.shape,
        )
        .unwrap()
        .join()
        .unwrap();
        String::from_utf8(sink.bytes()).unwrap()
    }

    /// What a terminal would show, though rows settle before the next redraw:
    /// a row curses erases right after is not given back first, a move up
    /// past given-back rows overwrites none of those still pending, and the
    /// end leaves no empty rows.
    #[test]
    fn rendering_settles_early_only_what_stays() {
        assert_eq!(
            rendered(b"INFO: x\n(11:31:13) \r\n\r\x1b[1A\x1b[KINFO: y\n"),
            "INFO: x\nINFO: y\n",
            "a timestamp-only row curses erases"
        );
        assert_eq!(
            rendered(b"step 1\r\n\r\x1b[1A\x1b[Kstep 2\r\n"),
            "step 1\nstep 2\n",
            "a tool redrawing its own line shows both states, settled before the redraw"
        );
        assert_eq!(
            rendered(b"INFO: From x:\na\nb\r\x1b[1A\x1b[Kc\n"),
            "INFO: From x:\na\nc\nb\n",
            "a was shown already; b is not lost"
        );
        assert_eq!(
            rendered(b"INFO: done\n\x1b[32m[3 / 3]\x1b[0m running\n    a\n\r\x1b[1A\x1b[K\r\x1b[1A\x1b[K\r\x1b[1A\x1b[KINFO: Build completed\n\x1b[0m"),
            "INFO: done\nINFO: Build completed\n",
            "no blank rows at the end; every row ends its own colour"
        );
        assert_eq!(
            rendered(
                b"ERROR: x\n\x1b[32m[1 / 2]\x1b[0m Executing //:slow; 1s\n    more\n\nBazel caught interrupt signal\n\x1b[31m\x1b[1mFAILED:\x1b[0m \n"
            ),
            "ERROR: x\n\x1b[32m[1 / 2]\x1b[0m Executing //:slow; 1s\n    more\n\nBazel caught interrupt signal\n",
            "an area left mid-stream stays, as a terminal shows it; the status line at the end does not"
        );
        assert_eq!(
            rendered(
                b"INFO: x\n\x1b[31m\x1b[1mFAILED:\x1b[0m \nWaiting for build events upload: Build Event Service 3s\n"
            ),
            "INFO: x\n",
            "a status line and the BES wait under it, both curses'"
        );
        assert_eq!(
            rendered(b"INFO: From x:\ntool 100%\r\x1b[0m\nnext\n"),
            "INFO: From x:\ntool 100%\nnext\n",
            "a colour change takes no columns"
        );
        assert_eq!(
            rendered(b"INFO: From x:\na\nb\n\x1b[999B\nc\n"),
            "INFO: From x:\na\nb\n\nc\n",
            "moving down adds no rows"
        );
        let huge = rendered(b"INFO: From x:\na\n\x1b[99999999999999A\x1b[18446744073709551615Bb\n");
        assert!(huge.contains("a\n") && huge.contains('b'), "{huge:?}");
        assert_eq!(
            rendered(b"INFO: From Testing //:t:\n a\n\n b\n c\n"),
            "INFO: From Testing //:t:\n a\n\n b\n c\n",
            "a blank row in the middle does not hold the rows after it"
        );
        assert_eq!(
            rendered(
                b"INFO: From Testing //:t:\nTraceback\n\x1b[32m[3 / 9]\x1b[0m Compiling\n  File \"t.py\", line 3\n    main()\nAssertionError\nINFO: done\n"
            ),
            "INFO: From Testing //:t:\nTraceback\n\x1b[32m[3 / 9]\x1b[0m Compiling\n  File \"t.py\", line 3\n    main()\nAssertionError\nINFO: done\n",
            "a test's output Bazel-coloured like progress, mid-stream, is all kept"
        );
    }

    /// Hidden, a curses console shows nothing, and its feed gets the lines
    /// and each progress area, through the pipeline and the forwarder.
    #[test]
    fn a_hidden_console_feeds_lines_and_progress() {
        let mut config = Config::new(Mode::Pty, vec![], Style::default());
        config.output = Output::Hidden;
        let pipeline = config.pipeline();
        let feed = pipeline.feed.clone().expect("a hidden console has a feed");
        let first = "\x1b[32mINFO: \x1b[0mone\r\n\x1b[32m[1 / 3]\x1b[0m 2 actions running\r\n    a.cc; 1s\r\n    b.cc; 2s\r\n\r\x1b[1A\x1b[K\r\x1b[1A\x1b[K\r\x1b[1A\x1b[K";
        let (read, write) = nix::unistd::pipe().unwrap();
        let mut write = std::fs::File::from(write);
        let sink = SharedSink::default();
        let mut forwarder = Forwarder::spawn_logged(
            Box::new(std::fs::File::from(read)),
            Box::new(sink.clone()),
            pipeline.chain,
            None,
            pipeline.shape,
        )
        .unwrap();
        std::io::Write::write_all(&mut write, first.as_bytes()).unwrap();
        // The area is in the feed while the console is open.
        let mut lines = vec![];
        let progress = (0..400)
            .find_map(|_| {
                let got = feed.drain();
                lines.extend(got.lines);
                if got.progress.is_empty() {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    None
                } else {
                    Some(got.progress)
                }
            })
            .unwrap_or_default();
        std::io::Write::write_all(&mut write, b"\x1b[32mINFO: \x1b[0mtwo\r\n").unwrap();
        drop(write);
        forwarder.join().unwrap();
        assert_eq!(sink.bytes(), b"", "nothing shown");
        assert_eq!(
            progress,
            vec!["[1 / 3] 2 actions running", "    a.cc; 1s", "    b.cc; 2s"]
        );
        lines.extend(feed.drain().lines);
        assert!(
            lines.contains(&"INFO: one".to_string()) && lines.contains(&"INFO: two".to_string()),
            "{lines:?}"
        );
    }

    /// A log is kept only when a stage can change what is shown; one that
    /// cannot be made is said to have failed, and the console still runs.
    #[test]
    fn a_log_that_cannot_be_made_is_reported() {
        let observe = Config::new(
            Mode::Pipe,
            vec![Stage::observe([("x".into(), "x".into())]).unwrap()],
            Style::default(),
        );
        assert!(!observe.keeps_log());
        let mut hidden = observe.clone();
        hidden.output = Output::Hidden;
        assert!(
            hidden.keeps_log(),
            "nothing shown: the log is the only copy"
        );
        let dir = tempfile::tempdir().unwrap();
        let taken = dir.path().join("console.log");
        std::fs::write(&taken, "an earlier one").unwrap();
        let config = Config::new(
            Mode::Pipe,
            vec![Stage::drop(["^INFO: x".to_string()]).unwrap()],
            Style::default(),
        )
        .with_log(log::Place::At(taken.clone()));
        assert!(config.keeps_log());
        let mut pipeline = config.pipeline();
        assert!(pipeline.log.is_none());
        let sink = SharedSink::default();
        Forwarder::spawn_logged(
            Box::new(std::io::Cursor::new(
                crate::console::line::tests::bazel("INFO: x\nINFO: y\n").into_bytes(),
            )),
            Box::new(sink.clone()),
            std::mem::take(&mut pipeline.chain),
            None,
            pipeline.shape,
        )
        .unwrap()
        .join()
        .unwrap();
        let out = ansi::text(&sink.bytes());
        assert!(
            out.contains("INFO: y") && out.contains("could not be saved"),
            "{out}"
        );
        assert_eq!(std::fs::read_to_string(&taken).unwrap(), "an earlier one");
    }

    /// No matching stage sees a row of Bazel's progress UI, whatever the
    /// mode: where curses is shown (redrawn rows), in plain lines rendered
    /// from it (the status line they let through), or in Bazel's own plain
    /// lines (its `[n / m]` lines); its messages it sees once each, and a
    /// tool's line shaped like progress (`[1 / 2] fake progress`) too.
    #[test]
    fn matching_stages_never_see_bazels_progress() {
        const PROGRESS: &str = r"\[\d+ / \d+\]|last test:|Analyzing:";
        let cases: [(&str, Mode, &[u8], Option<Output>); 3] = [
            (
                "curses",
                Mode::Pty,
                include_bytes!("testdata/pty_bazel8.txt"),
                None,
            ),
            (
                "lines",
                Mode::Pty,
                include_bytes!("testdata/pty_bazel8.txt"),
                Some(Output::Lines {
                    status_every: Some(Duration::ZERO),
                }),
            ),
            (
                "pipe",
                Mode::Pipe,
                include_bytes!("testdata/pipe_genrules.txt"),
                None,
            ),
        ];
        let mut messages = vec![];
        let progress = regex::Regex::new(PROGRESS).unwrap();
        for (name, mode, input, output) in cases {
            let spec = |id: &str, p: &str| {
                Stage::Aggregate(Box::new(
                    aggregate::Spec::new(id.into(), p, vec![], vec![], 0, 1).unwrap(),
                ))
            };
            let stages = vec![
                spec("progress", PROGRESS),
                spec("messages", "^INFO: "),
                spec("tool", r"\[\d+ / \d+\] fake progress"),
                Stage::observe([("progress".into(), PROGRESS.into())]).unwrap(),
            ];
            let mut config = Config::new(mode, stages, Style::default());
            if let Some(output) = output {
                config.output = output;
                config.pty = false;
            }
            let mut pipeline = config.pipeline();
            let sink = SharedSink::default();
            Forwarder::spawn_logged(
                Box::new(std::io::Cursor::new(input.to_vec())),
                Box::new(sink.clone()),
                std::mem::take(&mut pipeline.chain),
                None,
                pipeline.shape,
            )
            .unwrap()
            .join()
            .unwrap();
            let shown = ansi::text(&sink.bytes());
            let counts: Vec<u64> = pipeline
                .observations
                .aggregates()
                .iter()
                .map(|r| r["count"].as_u64().unwrap())
                .collect();
            assert_eq!(counts[0], counts[2], "{name}: only the tool's own lines");
            if name != "curses" {
                assert_eq!(
                    counts[1] as usize,
                    shown.lines().filter(|l| l.starts_with("INFO: ")).count(),
                    "{name}: each message once"
                );
            }
            messages.push(counts[1]);
            assert_eq!(
                pipeline.observations.matches().len() as u64,
                counts[2],
                "{name}"
            );
            if name == "lines" {
                assert!(
                    progress.is_match(&shown),
                    "the status line is shown, just not matched"
                );
            }
        }
        assert_eq!(
            messages[0], messages[1],
            "curses counts what plain lines show"
        );
    }

    /// Rendered lines carry colour per character: a move or an overwrite
    /// never cuts a colour sequence, never leaves one on for the next line,
    /// and never loses one a terminal would show; a flood of moves is cheap.
    #[test]
    fn rendered_colour_stays_on_its_own_characters() {
        for input in [
            &b"INFO: From x:\n\x1b[31mred\x1b[1Aabc\x1b[1BAll\n"[..],
            b"INFO: From x:\n\x1b[31mred\x1b[0m\x1b[1Aab\x1b[1BAll\n",
            b"INFO: From x:\n\x1b[31mfoo\nxx\x1b[0m\ryy\nzz\n",
        ] {
            let out = rendered(input);
            assert!(
                !out.contains("\x1b[3A") && !out.contains("\x1b[1A"),
                "{out:?}"
            );
            for l in out.lines() {
                assert!(
                    ansi::is_neutral(&ansi::controls(l.as_bytes())),
                    "colour left on at the end of {l:?} in {out:?}"
                );
            }
        }
        assert_eq!(
            rendered(b"INFO: From x:\n\x1b[31mfoo\nxx\x1b[0m\ryy\nzz\n"),
            "INFO: From x:\n\x1b[31mfoo\x1b[0m\nyy\nzz\n"
        );
        assert_eq!(
            rendered(b"INFO: From x:\n\x1b[31mred\rplain\n"),
            "INFO: From x:\n\x1b[31mplain\x1b[0m\n",
            "the colour in effect writes over the row"
        );
        let flood = b"\x1b[512A".repeat(6000);
        let started = std::time::Instant::now();
        rendered(&flood);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    /// The status line rendered lines let through goes through the stages,
    /// so a redacting `rewrite` masks it.
    #[test]
    fn a_status_line_goes_through_the_stages() {
        let mut config = Config::new(
            Mode::Pty,
            vec![Stage::rewrite("token=[a-z]+", "token=***".into()).unwrap()],
            Style::default(),
        );
        config.output = Output::Lines {
            status_every: Some(Duration::ZERO),
        };
        config.pty = false;
        let mut pipeline = config.pipeline();
        let sink = SharedSink::default();
        Forwarder::spawn_logged(
            Box::new(std::io::Cursor::new(
                b"INFO: x\n\x1b[32mFetching:\x1b[0m https://h/?token=secret\n\r\x1b[1A\x1b[KINFO: y\n".to_vec(),
            )),
            Box::new(sink.clone()),
            std::mem::take(&mut pipeline.chain),
            None,
            pipeline.shape,
        )
        .unwrap()
        .join()
        .unwrap();
        let out = ansi::text(&sink.bytes());
        assert!(
            out.contains("token=***") && !out.contains("secret"),
            "{out}"
        );
    }

    /// The default stages on rendered lines: a real curses console keeps
    /// every line of tool and test output, shows none of the progress area,
    /// and its errors are all told apart for the recap, since curses ends
    /// each action's output.
    #[test]
    fn recorded_curses_consoles_through_the_stages_as_lines() {
        for &(name, mode, input) in RECORDED {
            if mode != Mode::Pty {
                continue;
            }
            let mut config = defaults(mode, false);
            config.output = Output::Lines { status_every: None };
            let mut pipeline = config.pipeline();
            let sink = SharedSink::default();
            Forwarder::spawn_logged(
                Box::new(std::io::Cursor::new(input.to_vec())),
                Box::new(sink.clone()),
                std::mem::take(&mut pipeline.chain),
                None,
                pipeline.shape,
            )
            .unwrap()
            .join()
            .unwrap();
            let (before, after) = (ansi::text(input), ansi::text(&sink.bytes()));
            assert!(
                !sink.bytes().windows(4).any(|w| w == b"\x1b[1A"),
                "{name}: cursor movement"
            );
            for line in OUTPUT {
                if name == "pty_tool_redraw" && *line == "step 1" {
                    continue;
                }
                if before.contains(line) {
                    assert!(after.contains(line), "{name}: {line:?} lost in\n{after}");
                }
            }
            assert!(
                !after.contains("actions running"),
                "{name}: progress left in\n{after}"
            );
            if name == "pty_keep_going" {
                let recap = &after[after
                    .find("Errors Bazel reported")
                    .or_else(|| after.find("errors Bazel reported"))
                    .unwrap_or_else(|| panic!("no recap:\n{after}"))..];
                for e in ["//:e0", "//:e1", "//:e3", "//:e4"] {
                    assert!(recap.contains(e), "{name}: {e} not recapped:\n{recap}");
                }
            }
        }
    }

    /// Rendered as plain lines, a real curses console keeps every line of
    /// tool and test output and loses the progress area: no cursor movement,
    /// no `actions running`.
    #[test]
    fn recorded_curses_consoles_render_to_their_lines() {
        for &(name, mode, input) in RECORDED {
            if mode != Mode::Pty {
                continue;
            }
            let mut screen = render::Screen::new(None);
            let mut rows = screen.feed(input, None);
            rows.extend(screen.finish());
            let out = render::joined(&rows);
            assert!(
                !out.windows(4).any(|w| w == b"\x1b[1A") && !out.windows(3).any(|w| w == b"\x1b[K"),
                "{name}: cursor movement left in the output"
            );
            let (before, after) = (ansi::text(input), ansi::text(&out));
            for line in OUTPUT {
                // A tool's own redraw erases its line, as a terminal shows.
                if name == "pty_tool_redraw" && *line == "step 1" {
                    continue;
                }
                if before.contains(line) {
                    assert!(after.contains(line), "{name}: {line:?} lost in\n{after}");
                }
            }
            assert!(
                !after.contains("actions running"),
                "{name}: progress left in\n{after}"
            );
        }
    }

    /// The recap of a recorded console, in curses (as written) and in
    /// plain lines: each recapped error with the body Bazel wrote under it.
    #[test]
    fn a_recorded_consoles_recap_keeps_each_errors_body_in_either_output() {
        let cases: &[(&str, &[u8], &[&str])] = &[
            (
                "pty_keep_going",
                include_bytes!("testdata/pty_keep_going.txt"),
                &["err0", "err1", "err3"],
            ),
            (
                "pty_action_error_text",
                include_bytes!("testdata/pty_action_error_text.txt"),
                &["unused import \"os\"", "unused import \"sys\""],
            ),
        ];
        for &(name, input, body) in cases {
            for lines in [false, true] {
                let mut config = Config::new(
                    Mode::Pty,
                    vec![Stage::RecapErrors {
                        max: report::RECAP_MAX,
                        after: 0,
                    }],
                    Style::default(),
                );
                if lines {
                    config.output = Output::Lines { status_every: None };
                }
                let mut pipeline = config.pipeline();
                let sink = SharedSink::default();
                let mut forwarder = Forwarder::spawn(
                    Box::new(std::io::Cursor::new(input.to_vec())),
                    Box::new(sink.clone()),
                    std::mem::take(&mut pipeline.chain),
                )
                .unwrap();
                forwarder.join().unwrap();
                let after = ansi::text(&sink.bytes());
                let at = after
                    .find("INFO: Errors Bazel reported")
                    .or_else(|| after.find("INFO: The first"))
                    .unwrap_or_else(|| panic!("{name} (lines: {lines}): no recap in\n{after}"));
                let recap = &after[at..];
                for line in body {
                    assert!(
                        recap.contains(line),
                        "{name} (lines: {lines}): {line:?} not in the recap\n{recap}"
                    );
                }
                assert!(
                    !recap.contains("Use --sandbox_debug"),
                    "{name} (lines: {lines}): the sandbox hint in\n{recap}"
                );
            }
        }
    }

    #[test]
    fn recorded_consoles_keep_all_action_and_test_output() {
        for &(name, mode, input) in RECORDED {
            for agent in [false, true] {
                let mut pipeline = defaults(mode, agent).pipeline();
                let sink = SharedSink::default();
                let mut forwarder = Forwarder::spawn(
                    Box::new(std::io::Cursor::new(input.to_vec())),
                    Box::new(sink.clone()),
                    std::mem::take(&mut pipeline.chain),
                )
                .unwrap();
                forwarder.join().unwrap();
                let (before, after) = (ansi::text(input), ansi::text(&sink.bytes()));
                // The console itself, without the reports after it, whose
                // quotes would pass for lines that survived.
                let report = [
                    "INFO: Aspect hid",
                    "INFO: The first",
                    "INFO: Errors Bazel reported",
                ]
                .iter()
                .filter_map(|r| after.find(r))
                .min()
                .unwrap_or(after.len());
                let shown = &after[..report];
                for line in OUTPUT {
                    assert_eq!(
                        shown.matches(line).count(),
                        before.matches(line).count(),
                        "{name} (agent: {agent}): {line:?} lost in\n{after}"
                    );
                }
                if agent
                    && mode == Mode::Pipe
                    && name != "pipe_no_color"
                    && !name.contains("streamed")
                {
                    let phase = |t: &str| {
                        t.lines()
                            .filter(|l| l.starts_with("Loading:") || l.starts_with("Analyzing:"))
                            .count()
                    };
                    assert!(
                        phase(&before) == 0 || phase(&after) < phase(&before),
                        "{name}: the agent profile drops bazel's phase lines:\n{after}"
                    );
                }
                if name.starts_with("pipe_canonical") || name == "pty_bazel8_mixed" {
                    assert!(
                        shown.contains("indicated that a canonical"),
                        "{name} (agent: {agent}): nothing is dropped by default:\n{after}"
                    );
                }
                if name == "pipe_analysis_errors" {
                    assert!(
                        after.contains("more like: WARNING: errors encountered while analyzing"),
                        "{name} (agent: {agent}): analysis errors leave the warnings after them collapsible:\n{after}"
                    );
                }
                // A Starlark `print()` shows every value it printed: these
                // differ, so none is folded.
                let noisy = |t: &str| t.matches("noisy debug").count();
                assert_eq!(
                    noisy(shown),
                    noisy(&before),
                    "{name} (agent: {agent}): every print() value is shown:\n{after}"
                );
            }
        }
    }
}
