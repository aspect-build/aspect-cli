//! Parse AXL files ahead of their evaluation, on a few threads.
//!
//! Reading and parsing are about a third of start-up evaluation, and unlike
//! evaluation they depend on nothing but the file, so they can run in any
//! order and in parallel. [`with_prefetch`] starts workers that walk the
//! `load()` graph from a set of roots, parsing each file and queueing the
//! files it loads, while the caller evaluates on its own thread as usual.
//! When evaluation reaches a file it calls [`Prefetch::take`], which hands
//! back the parsed AST, waits for a worker that is parsing it, or — when no
//! worker has started on it yet — lets the evaluating thread parse it itself
//! rather than wait behind the queue.
//!
//! Evaluation order, scoping and diagnostics stay the loader's own. A file
//! that fails to read, parse or resolve here is handed back as not
//! prefetched, so the loader reads and parses it itself and reports the
//! failure in the usual place. `@std` and `@bazel` files are embedded and
//! tiny, and are not prefetched.
//!
//! Only phase 1 is prefetched. Its files run no side effects at module
//! level, so a file cannot change between its parse and its evaluation; a
//! config can write a file a later config loads.
//!
//! A panic on a worker is caught where panics unwind; an optimized Bazel
//! build aborts on one, as it would for a panic anywhere else.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use starlark::syntax::{AstModule, Dialect};

use super::load::{Target, resolve};
use super::load_path::LoadPath;
use crate::module::Mod;

/// Upper bound on worker threads. Parsing a phase's files is a few tens of
/// milliseconds of work, and the load graph's depth, not the core count,
/// limits how far ahead workers can get.
const MAX_WORKERS: usize = 8;

/// Prefetch state shared between the workers and the evaluating thread.
pub(crate) struct Prefetch<'m> {
    modules: &'m [Mod],
    dialect: Dialect,
    state: Mutex<State<'m>>,
    wake: Condvar,
}

#[derive(Default)]
struct State<'m> {
    /// Files queued for a worker, popped from the end. [`Prefetch::queue`]
    /// pushes each batch reversed, so the walk is roughly depth-first in the
    /// order evaluation will ask for the files.
    pending: Vec<(&'m Mod, PathBuf)>,
    /// Every path ever queued, so a file is parsed once.
    seen: HashSet<PathBuf>,
    /// Paths a worker is parsing now.
    in_flight: HashSet<PathBuf>,
    /// Finished paths; `None` for one that failed and is left to the loader.
    ready: HashMap<PathBuf, Option<AstModule>>,
    /// Set when the caller's evaluation is over; workers then exit.
    done: bool,
}

/// What [`Prefetch::take`] found for a path.
pub(crate) enum Taken {
    /// Parsed and ready to evaluate.
    Parsed(AstModule),
    /// Not prefetched: the caller reads and parses it itself.
    Absent,
}

impl std::fmt::Debug for Prefetch<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prefetch").finish_non_exhaustive()
    }
}

impl<'m> Prefetch<'m> {
    /// The AST for `path`, evaluated under `scope`; see the module docs.
    pub(crate) fn take(&self, scope: &'m Mod, path: &Path) -> Taken {
        let mut state = self.lock();
        loop {
            if let Some(ast) = state.ready.remove(path) {
                return ast.map_or(Taken::Absent, Taken::Parsed);
            }
            if state.in_flight.contains(path) {
                state = self.wait(state);
                continue;
            }
            let Some(i) = state.pending.iter().position(|(_, p)| p == path) else {
                return Taken::Absent;
            };
            // No worker has reached it: parse it here, and queue its loads.
            state.pending.remove(i);
            drop(state);
            let ast = self.parse(path);
            if let Some(ast) = &ast {
                self.queue(self.loads(scope, path, ast));
            }
            return ast.map_or(Taken::Absent, Taken::Parsed);
        }
    }

    fn lock(&self) -> MutexGuard<'_, State<'m>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait<'g>(&self, guard: MutexGuard<'g, State<'m>>) -> MutexGuard<'g, State<'m>> {
        self.wake
            .wait(guard)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Read and parse `path`; `None` for any failure, so the loader redoes
    /// it and reports it. Anything but a regular file is left to the loader,
    /// so a FIFO or device a load names cannot block a worker.
    fn parse(&self, path: &Path) -> Option<AstModule> {
        if !fs::metadata(path).ok()?.is_file() {
            return None;
        }
        let raw = fs::read_to_string(path).ok()?;
        AstModule::parse(&path.to_string_lossy(), raw, &self.dialect).ok()
    }

    /// Release the workers.
    fn finish(&self) {
        self.lock().done = true;
        self.wake.notify_all();
    }

    /// The files `ast` loads that are evaluated from disk under a `Mod`.
    fn loads(&self, scope: &'m Mod, path: &Path, ast: &AstModule) -> Vec<(&'m Mod, PathBuf)> {
        ast.loads()
            .into_iter()
            .filter_map(|load| {
                let load_path = LoadPath::try_from(load.module_id).ok()?;
                match resolve(self.modules, &load_path, Some(scope), Some(path)) {
                    Ok((dep, Target::Module(dep_scope))) => Some((dep_scope, dep)),
                    _ => None,
                }
            })
            .collect()
    }

    fn queue(&self, files: Vec<(&'m Mod, PathBuf)>) {
        let mut state = self.lock();
        for (scope, path) in files.into_iter().rev() {
            if state.seen.insert(path.clone()) {
                state.pending.push((scope, path));
            }
        }
        drop(state);
        self.wake.notify_all();
    }

    fn work(&self) {
        loop {
            let (scope, path) = {
                let mut state = self.lock();
                loop {
                    if state.done {
                        return;
                    }
                    if let Some(job) = state.pending.pop() {
                        state.in_flight.insert(job.1.clone());
                        break job;
                    }
                    state = self.wait(state);
                }
            };
            // Whatever happens to the job, the path leaves `in_flight`, or a
            // `take` waiting on it would wait forever.
            let ast = catch_unwind(AssertUnwindSafe(|| {
                let ast = self.parse(&path)?;
                self.queue(self.loads(scope, &path, &ast));
                Some(ast)
            }))
            .ok()
            .flatten();
            let mut state = self.lock();
            state.in_flight.remove(&path);
            state.ready.insert(path, ast);
            drop(state);
            self.wake.notify_all();
        }
    }
}

/// Releases the workers when dropped, so a panicking `body` unwinds through
/// the thread scope instead of leaving it waiting on idle workers.
struct Finish<'a, 'm>(&'a Prefetch<'m>);

impl Drop for Finish<'_, '_> {
    fn drop(&mut self) {
        self.0.finish();
    }
}

/// Run `body` while workers prefetch every file reachable from `roots`;
/// `body` reaches the results through the [`Prefetch`] it is given, or gets
/// `None` when there is nothing to run ahead on (no roots, or a single core).
/// The workers stop when `body` returns.
pub(crate) fn with_prefetch<'m, T>(
    modules: &'m [Mod],
    dialect: &Dialect,
    roots: Vec<(&'m Mod, PathBuf)>,
    body: impl FnOnce(Option<Arc<Prefetch<'m>>>) -> T,
) -> T {
    // The evaluating thread is busy with `body`, so it is not counted.
    let workers = std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1))
        .unwrap_or(0)
        .min(MAX_WORKERS);
    with_workers(modules, dialect, roots, workers, body)
}

/// [`with_prefetch`] with up to `workers` threads.
fn with_workers<'m, T>(
    modules: &'m [Mod],
    dialect: &Dialect,
    roots: Vec<(&'m Mod, PathBuf)>,
    workers: usize,
    body: impl FnOnce(Option<Arc<Prefetch<'m>>>) -> T,
) -> T {
    if roots.is_empty() || workers == 0 {
        return body(None);
    }
    let prefetch = Arc::new(Prefetch {
        modules,
        dialect: dialect.clone(),
        state: Mutex::new(State::default()),
        wake: Condvar::new(),
    });
    prefetch.queue(roots);
    std::thread::scope(|scope| {
        // A thread the OS refuses is one fewer worker; `take` parses whatever
        // no worker reaches.
        for _ in 0..workers {
            let prefetch = prefetch.clone();
            let _ = std::thread::Builder::new()
                .name("axl-prefetch".into())
                .spawn_scoped(scope, move || prefetch.work());
        }
        let _finish = Finish(&prefetch);
        body(Some(prefetch.clone()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::api;

    /// A module rooted at a temp dir holding `files`, each `(relative path,
    /// source)`.
    fn module(files: &[(&str, &str)]) -> (tempfile::TempDir, Mod) {
        let dir = tempfile::tempdir().unwrap();
        for (rel, src) in files {
            fs::write(dir.path().join(rel), src).unwrap();
        }
        let root = dir.path().to_path_buf();
        (dir, Mod::new(root.clone(), "test".to_string(), root))
    }

    /// Every file reachable through relative loads comes back parsed, and a
    /// file that does not parse comes back absent so the loader reports it.
    #[test]
    fn walks_the_load_graph_and_leaves_failures_to_the_loader() {
        let (dir, r#mod) = module(&[
            (
                "a.axl",
                "load(\"./b.axl\", \"b\")\nload(\"./broken.axl\", \"x\")\na = b\n",
            ),
            ("b.axl", "load(\"./c.axl\", \"c\")\nb = c\n"),
            ("c.axl", "c = 1\n"),
            ("broken.axl", "def (\n"),
        ]);
        let path = |name: &str| dir.path().join(name);
        let modules = [];
        with_workers(
            &modules,
            &api::dialect(),
            vec![(&r#mod, path("a.axl"))],
            2,
            |p| {
                let p = p.expect("prefetch runs");
                for name in ["a.axl", "b.axl", "c.axl"] {
                    assert!(
                        matches!(p.take(&r#mod, &path(name)), Taken::Parsed(_)),
                        "{name} was not prefetched"
                    );
                }
                assert!(matches!(p.take(&r#mod, &path("broken.axl")), Taken::Absent));
                // A path is handed out once; a second take is the loader's.
                assert!(matches!(p.take(&r#mod, &path("c.axl")), Taken::Absent));
                // A file nothing loads was never queued.
                assert!(matches!(p.take(&r#mod, &path("other.axl")), Taken::Absent));
            },
        );
    }

    /// A load naming something other than a regular file is left to the
    /// loader rather than read by a worker.
    #[test]
    fn leaves_non_files_to_the_loader() {
        let (dir, r#mod) = module(&[("a.axl", "load(\"./dir.axl\", \"x\")\n")]);
        fs::create_dir(dir.path().join("dir.axl")).unwrap();
        let path = |name: &str| dir.path().join(name);
        with_workers(
            &[],
            &api::dialect(),
            vec![(&r#mod, path("a.axl"))],
            2,
            |p| {
                let p = p.expect("prefetch runs");
                assert!(matches!(p.take(&r#mod, &path("a.axl")), Taken::Parsed(_)));
                assert!(matches!(p.take(&r#mod, &path("dir.axl")), Taken::Absent));
            },
        );
    }

    /// A file loaded from two roots is queued once and handed out once. Takes
    /// follow evaluation order: a file's loads are queued before the file is
    /// handed out, so `c.axl` is known once `a.axl` has been taken.
    #[test]
    fn a_shared_load_is_parsed_once() {
        let (dir, r#mod) = module(&[
            ("a.axl", "load(\"./c.axl\", \"c\")\n"),
            ("b.axl", "load(\"./c.axl\", \"c\")\n"),
            ("c.axl", "c = 1\n"),
        ]);
        let path = |name: &str| dir.path().join(name);
        let roots = vec![(&r#mod, path("a.axl")), (&r#mod, path("b.axl"))];
        with_workers(&[], &api::dialect(), roots, 2, |p| {
            let p = p.expect("prefetch runs");
            assert!(matches!(p.take(&r#mod, &path("a.axl")), Taken::Parsed(_)));
            assert!(matches!(p.take(&r#mod, &path("c.axl")), Taken::Parsed(_)));
            assert!(matches!(p.take(&r#mod, &path("b.axl")), Taken::Parsed(_)));
            assert!(matches!(p.take(&r#mod, &path("c.axl")), Taken::Absent));
        });
    }

    /// `take` on a file a worker is parsing blocks until the worker's
    /// result lands.
    #[test]
    fn take_waits_for_a_file_in_flight() {
        use std::sync::mpsc;
        use std::time::Duration;
        let (dir, r#mod) = module(&[("a.axl", "a = 1\n")]);
        let path = dir.path().join("a.axl");
        let prefetch = Prefetch {
            modules: &[],
            dialect: api::dialect(),
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        };
        prefetch.lock().in_flight.insert(path.clone());
        let (prefetch, path, r#mod) = (&prefetch, &path, &r#mod);
        std::thread::scope(|scope| {
            let (done, taken) = mpsc::channel();
            scope.spawn(move || {
                let parsed = matches!(prefetch.take(r#mod, path), Taken::Parsed(_));
                done.send(parsed).unwrap();
            });
            assert!(
                taken.recv_timeout(Duration::from_millis(50)).is_err(),
                "take returned while the file was in flight"
            );
            let ast = prefetch.parse(path);
            let mut state = prefetch.lock();
            state.in_flight.remove(path);
            state.ready.insert(path.clone(), ast);
            drop(state);
            prefetch.wake.notify_all();
            assert!(taken.recv().unwrap());
        });
    }

    /// A `@dep//` load is resolved against the dep module and walked in its
    /// scope, so the dep's own relative loads are found too.
    #[test]
    fn follows_loads_into_another_module() {
        let (dir, r#mod) = module(&[("a.axl", "load(\"@dep//lib.axl\", \"x\")\n")]);
        let (dep_dir, _) = module(&[
            ("lib.axl", "load(\"./inner.axl\", \"y\")\nx = 1\n"),
            ("inner.axl", "y = 1\n"),
        ]);
        let dep_root = dep_dir.path().to_path_buf();
        let modules = [Mod::new(
            dep_root.clone(),
            "dep".to_string(),
            dep_root.clone(),
        )];
        let roots = vec![(&r#mod, dir.path().join("a.axl"))];
        with_workers(&modules, &api::dialect(), roots, 2, |p| {
            let p = p.expect("prefetch runs");
            assert!(matches!(
                p.take(&r#mod, &dir.path().join("a.axl")),
                Taken::Parsed(_)
            ));
            let dep = &modules[0];
            assert!(matches!(
                p.take(dep, &dep_root.join("lib.axl")),
                Taken::Parsed(_)
            ));
            assert!(matches!(
                p.take(dep, &dep_root.join("inner.axl")),
                Taken::Parsed(_)
            ));
        });
    }

    /// With no workers, nothing is installed and the loader parses as usual.
    #[test]
    fn no_workers_installs_nothing() {
        let (dir, r#mod) = module(&[("a.axl", "a = 1\n")]);
        let roots = vec![(&r#mod, dir.path().join("a.axl"))];
        assert!(with_workers(&[], &api::dialect(), roots, 0, |p| p.is_none()));
    }

    /// A panic in the body propagates rather than hanging on the workers.
    #[test]
    fn a_panicking_body_does_not_hang() {
        let (dir, r#mod) = module(&[("a.axl", "a = 1\n")]);
        let roots = vec![(&r#mod, dir.path().join("a.axl"))];
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            with_workers(&[], &api::dialect(), roots, 2, |_| -> () {
                panic!("body failed")
            })
        }));
        assert!(result.is_err());
    }

    /// With no workers at all, the evaluating thread parses queued files
    /// itself and still follows their loads.
    #[test]
    fn take_parses_a_queued_file_itself() {
        let (dir, r#mod) = module(&[
            ("a.axl", "load(\"./b.axl\", \"b\")\na = b\n"),
            ("b.axl", "b = 1\n"),
        ]);
        let prefetch = Prefetch {
            modules: &[],
            dialect: api::dialect(),
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        };
        prefetch.queue(vec![(&r#mod, dir.path().join("a.axl"))]);
        assert!(matches!(
            prefetch.take(&r#mod, &dir.path().join("a.axl")),
            Taken::Parsed(_)
        ));
        assert!(matches!(
            prefetch.take(&r#mod, &dir.path().join("b.axl")),
            Taken::Parsed(_)
        ));
    }
}
