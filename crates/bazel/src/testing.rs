use std::time::Duration;

/// Run `f` on its own thread and wait at most `timeout` for it. `None` leaves
/// whatever `f` was doing in flight, so a test that hangs fails instead of
/// stalling the suite.
pub fn with_timeout<F, R>(timeout: Duration, f: F) -> Option<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(timeout).ok()
}
