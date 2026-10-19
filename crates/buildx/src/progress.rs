//! The progress line of a running build, on stderr.
//!
//! Bazel's own `ExecutionProgressReceiver` is handed to `buildArtifacts`: it
//! counts every action that completed, cached and remote hits included, and the
//! actions enqueued so far, which is the `[n / m]` of Bazel's console. A thread
//! reads that once a second and writes one line; nothing runs per action. The
//! total grows as analysis discovers actions, so the line says `n/m-so-far`.

use crate::bindings::v9_0_1 as bazel;
use blazon_rt::{Global, vm};
use std::io::{IsTerminal, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Builds shorter than this print nothing.
const QUIET: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(200);

struct Shared {
    receiver: Global<bazel::ExecutionProgressReceiver<'static>>,
    errors: Global<bazel::StoredEventHandler<'static>>,
    started: Instant,
    terminal: bool,
    stop: AtomicBool,
    printed: AtomicBool,
}

pub struct Progress {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

/// `[1,234 / 10,101]` as (1234, 10101).
pub(crate) fn parse(text: &str) -> Option<(u64, u64)> {
    let inner = text.trim().strip_prefix('[')?.strip_suffix(']')?;
    let (done, total) = inner.split_once('/')?;
    let number = |s: &str| s.trim().replace(',', "").parse().ok();
    Some((number(done)?, number(total)?))
}

impl Shared {
    fn counts(&self) -> Option<(u64, u64, usize)> {
        let mut env = vm::env().ok()?;
        let (done, total) = parse(&self.receiver.get(&mut env).ok()?.progress_string().ok()?)?;
        let failed = self.failed(&mut env);
        Some((done, total, failed))
    }

    /// Errors Bazel reported so far: one per failed action.
    fn failed(&self, env: &mut jni::JNIEnv<'_>) -> usize {
        let Ok(handler) = self.errors.get(env) else {
            return 0;
        };
        let Ok(events) = handler.events() else {
            return 0;
        };
        let mut n = 0;
        for e in (&events.upcast::<bazel::Iterable<bazel::Event>>())
            .into_iter()
            .flatten()
        {
            if e.kind().is_ok_and(|k| k.to_string() == "ERROR") {
                n += 1;
            }
        }
        n
    }

    fn line(&self, final_line: bool) {
        let Some((done, total, failed)) = self.counts() else {
            return;
        };
        let secs = self.started.elapsed().as_secs();
        let mut text = format!("buildx: [{done}/{total}] actions, {secs}s");
        if final_line && failed > 0 {
            text.push_str(&format!(", {failed} failed"));
        }
        let mut err = std::io::stderr().lock();
        if self.terminal {
            let end = if final_line { "\n" } else { "" };
            let _ = write!(err, "\r{text}\x1b[K{end}");
        } else {
            let _ = writeln!(err, "{text}");
        }
        let _ = err.flush();
        self.printed.store(true, Ordering::Relaxed);
    }
}

impl Progress {
    pub fn start(
        receiver: Global<bazel::ExecutionProgressReceiver<'static>>,
        errors: Global<bazel::StoredEventHandler<'static>>,
    ) -> Progress {
        let shared = Arc::new(Shared {
            receiver,
            errors,
            started: Instant::now(),
            terminal: std::io::stderr().is_terminal(),
            stop: AtomicBool::new(false),
            printed: AtomicBool::new(false),
        });
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("buildx-progress".into())
            .spawn(move || {
                let mut last = Instant::now();
                while !worker.stop.load(Ordering::Relaxed) {
                    std::thread::park_timeout(TICK);
                    if worker.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    if worker.started.elapsed() >= QUIET && last.elapsed() >= Duration::from_secs(1)
                    {
                        worker.line(false);
                        last = Instant::now();
                    }
                }
            })
            .ok();
        Progress { shared, thread }
    }

    /// Stops the line; a build that ran long enough ends with a final one.
    pub fn finish(mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
        if self.shared.printed.load(Ordering::Relaxed) || self.shared.started.elapsed() >= QUIET {
            self.shared.line(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn reads_bazels_progress_string() {
        assert_eq!(parse("[5,733 / 10,101]"), Some((5733, 10101)));
        assert_eq!(parse("[0 / 0]"), Some((0, 0)));
        assert_eq!(parse("nonsense"), None);
    }
}
