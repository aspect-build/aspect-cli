//! [`crate::Backend::Watchman`]: out-of-process watcher via the `watchman`
//! CLI, assumed to be on `PATH`.
//!
//! Per root we resolve the watchman watch with `watch-project`, snapshot the
//! clock, then hold a persistent `watchman --json-command --persistent`
//! process whose subscription streams JSON PDUs on stdout. A reader thread
//! feeds them into the shared [`Pending`] debouncer.
//!
//! Watches registered with the daemon are intentionally never `watch-del`ed:
//! watchman shares watches across clients, so deleting one could break other
//! tools watching the same project root.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use serde_json::{Value, json};

use crate::debounce::Pending;
use crate::{ChangeBatch, ChangeKind, Subscription, WatchConfig, WatchError, ignored, non_empty};

pub(crate) fn subscribe(config: WatchConfig) -> Result<Box<dyn Subscription>, WatchError> {
    let pending = Arc::new(Mutex::new(Pending::default()));
    let error = Arc::new(Mutex::new(None));
    let shutdown = Arc::new(AtomicBool::new(false));
    let name = format!("fswatch-{}", std::process::id());
    let (child, reader) =
        subscribe_root(&config.root, &name, &config, &pending, &error, &shutdown)?;
    Ok(Box::new(WatchmanSubscription {
        child: Some(child),
        reader: Some(reader),
        name,
        pending,
        error,
        shutdown,
        config,
    }))
}

fn subscribe_root(
    root: &PathBuf,
    name: &str,
    config: &WatchConfig,
    pending: &Arc<Mutex<Pending>>,
    error: &Arc<Mutex<Option<String>>>,
    shutdown: &Arc<AtomicBool>,
) -> Result<(Child, JoinHandle<()>), WatchError> {
    // watch-project requires an absolute path and realpaths it, so hand it
    // the canonical form — but keep reporting events under the caller-spelled
    // root, or a symlinked root (e.g. /tmp on macOS) would produce paths that
    // bypass the caller's ignore prefixes and confuse consumers.
    let canonical = root
        .canonicalize()
        .map_err(|e| WatchError::Backend(format!("cannot canonicalize {root:?}: {e}")))?;
    let project = run_oneshot(&json!(["watch-project", canonical]))?;
    let watch = project
        .get("watch")
        .and_then(Value::as_str)
        .ok_or_else(|| WatchError::Backend(format!("watch-project returned no watch: {project}")))?
        .to_string();
    let relative_path = project.get("relative_path").and_then(Value::as_str);
    // Events arrive relative to the watch (plus relative_root), which is the
    // canonical tree; re-root them at the path the caller asked to watch.
    let base = root.clone();
    // Subscribe from the current clock so watchman does not replay the entire
    // existing tree as an initial batch. sync_timeout makes the clock wait for
    // the initial crawl — an unsynchronized clock could predate it and replay
    // the whole tree anyway.
    let clock = run_oneshot(&json!(["clock", watch, {"sync_timeout": 10000}]))?
        .get("clock")
        .and_then(Value::as_str)
        .ok_or_else(|| WatchError::Backend("clock returned no clock".to_string()))?
        .to_string();

    let mut params = json!({
        "fields": ["name", "exists", "new"],
        "since": clock,
        // A fresh instance is handled as a rescan; without this the daemon
        // would replay the entire tree as one giant PDU we'd only discard.
        "empty_on_fresh_instance": true,
    });
    if let Some(rel) = relative_path {
        params["relative_root"] = json!(rel);
    }
    let command = json!(["subscribe", watch, name, params]);

    let mut child = Command::new("watchman")
        .args([
            "--json-command",
            "--persistent",
            "--server-encoding=json",
            "--no-pretty",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| WatchError::Backend(format!("failed to spawn watchman: {e}")))?;
    // Keep stdin owned by the child so the pipe stays open for its lifetime.
    let stdin = child.stdin.as_mut().expect("stdin is piped");
    if let Err(e) = writeln!(stdin, "{command}") {
        let _ = child.kill();
        let _ = child.wait();
        return Err(WatchError::Backend(format!(
            "failed to send subscribe: {e}"
        )));
    }
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut reader = BufReader::new(stdout);

    // The first PDU is the subscribe acknowledgement. Validating it here (not
    // in the reader thread) makes a rejected subscription fail subscribe(),
    // so Backend::Auto can fall back to notify instead of dying later.
    if let Err(e) = read_subscribe_ack(&mut reader, name) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(e);
    }

    let ignore_prefixes = config.ignore_prefixes.clone();
    let pending = Arc::clone(pending);
    let error = Arc::clone(error);
    let shutdown = Arc::clone(shutdown);
    let reader = std::thread::spawn(move || {
        read_events(reader, base, ignore_prefixes, &pending, &error, &shutdown);
    });
    Ok((child, reader))
}

/// Run a one-shot watchman command and return its decoded response.
fn run_oneshot(command: &Value) -> Result<Value, WatchError> {
    let output = Command::new("watchman")
        .args(["--json-command", "--output-encoding=json", "--no-pretty"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child
                .stdin
                .as_mut()
                .expect("stdin is piped")
                .write_all(command.to_string().as_bytes())?;
            child.wait_with_output()
        })
        .map_err(|e| WatchError::Backend(format!("failed to run watchman: {e}")))?;
    if !output.status.success() {
        return Err(WatchError::Backend(format!(
            "watchman {command} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let response: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| WatchError::Backend(format!("bad watchman response: {e}")))?;
    if let Some(err) = response.get("error").and_then(Value::as_str) {
        return Err(WatchError::Backend(format!("watchman {command}: {err}")));
    }
    Ok(response)
}

/// Read the first PDU after `subscribe` and require a non-error
/// acknowledgement (`{"subscribe": <name>}`).
fn read_subscribe_ack(reader: &mut impl BufRead, expected: &str) -> Result<(), WatchError> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| WatchError::Backend(format!("failed to read subscribe ack: {e}")))?;
    let ack: Value = serde_json::from_str(&line)
        .map_err(|e| WatchError::Backend(format!("bad subscribe ack: {e}")))?;
    if let Some(err) = ack.get("error").and_then(Value::as_str) {
        return Err(WatchError::Backend(format!("subscribe rejected: {err}")));
    }
    if ack.get("subscribe").and_then(Value::as_str) != Some(expected) {
        return Err(WatchError::Backend(format!(
            "unexpected first PDU after subscribe: {ack}"
        )));
    }
    Ok(())
}

fn read_events(
    stdout: impl BufRead,
    base: PathBuf,
    ignore_prefixes: Vec<PathBuf>,
    pending: &Mutex<Pending>,
    error: &Mutex<Option<String>>,
    shutdown: &AtomicBool,
) {
    for line in stdout.lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                error
                    .lock()
                    .unwrap()
                    .get_or_insert(format!("failed to read watchman response: {e}"));
                return;
            }
        };
        let value = match serde_json::from_str::<Value>(&line) {
            Ok(value) => value,
            Err(e) => {
                error
                    .lock()
                    .unwrap()
                    .get_or_insert(format!("bad watchman response: {e}"));
                return;
            }
        };
        if let Some(err) = value.get("error").and_then(Value::as_str) {
            error.lock().unwrap().get_or_insert(err.to_string());
            return;
        }
        // Non-unilateral PDUs carry no subscription field.
        if value.get("subscription").is_none() {
            continue;
        }
        if value.get("canceled").and_then(Value::as_bool) == Some(true) {
            // The watch was deleted from the daemon; this subscription is
            // permanently dead. Surface it instead of going silent.
            error
                .lock()
                .unwrap()
                .get_or_insert("watchman subscription canceled".to_string());
            return;
        }
        let mut pending = pending.lock().unwrap();
        if value.get("is_fresh_instance").and_then(Value::as_bool) == Some(true) {
            // The daemon restarted; per-file deltas are untrustworthy.
            pending.record_rescan();
            continue;
        }
        let Some(files) = value.get("files").and_then(Value::as_array) else {
            continue;
        };
        for file in files {
            let Some(name) = file.get("name").and_then(Value::as_str) else {
                continue;
            };
            let path = base.join(name);
            if ignored(&path, &ignore_prefixes) {
                continue;
            }
            let exists = file.get("exists").and_then(Value::as_bool).unwrap_or(true);
            let new = file.get("new").and_then(Value::as_bool).unwrap_or(false);
            let kind = match (exists, new) {
                (false, _) => ChangeKind::Removed,
                (true, true) => ChangeKind::Created,
                (true, false) => ChangeKind::Modified,
            };
            pending.record(path, kind);
        }
    }
    // EOF without a shutdown means the daemon or connection died under us.
    if !shutdown.load(Ordering::Relaxed) {
        error
            .lock()
            .unwrap()
            .get_or_insert("watchman connection closed unexpectedly".to_string());
    }
}

struct WatchmanSubscription {
    child: Option<Child>,
    reader: Option<JoinHandle<()>>,
    name: String,
    pending: Arc<Mutex<Pending>>,
    error: Arc<Mutex<Option<String>>>,
    shutdown: Arc<AtomicBool>,
    config: WatchConfig,
}

impl WatchmanSubscription {
    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Subscription for WatchmanSubscription {
    fn try_recv(&mut self) -> Result<Option<ChangeBatch>, WatchError> {
        if let Some(err) = self.error.lock().unwrap().take() {
            return Err(WatchError::Backend(err));
        }
        Ok(self
            .pending
            .lock()
            .unwrap()
            .try_take(self.config.debounce, self.config.max_delay)
            .and_then(non_empty))
    }

    fn drain(&mut self) {
        // Closing and joining the old subscription fences its reader. The new
        // subscription snapshots a synchronized clock, so changes preceding
        // this drain are not replayed into the new stream.
        self.stop();
        self.pending.lock().unwrap().clear();
        self.error.lock().unwrap().take();
        self.shutdown.store(false, Ordering::Relaxed);
        match subscribe_root(
            &self.config.root,
            &self.name,
            &self.config,
            &self.pending,
            &self.error,
            &self.shutdown,
        ) {
            Ok((child, reader)) => {
                self.child = Some(child);
                self.reader = Some(reader);
            }
            Err(e) => {
                let WatchError::Backend(message) = e;
                self.error.lock().unwrap().replace(message);
            }
        }
    }
}

impl Drop for WatchmanSubscription {
    fn drop(&mut self) {
        self.stop();
    }
}
