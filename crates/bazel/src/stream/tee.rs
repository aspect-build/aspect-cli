//! Copies of a stream written as it is read: a [`MultiTeeReader`] passes
//! every byte it reads to its [`Tee`]s, so a reader that follows Bazel's file
//! writes each caller's copy without a second pass.

use std::fs::File;
use std::io;
use std::io::{BufWriter, Read, Write};

/// One copy a reader writes. A copy that cannot be created, written, flushed or
/// copied again ([`MultiTeeReader::recopy`]) stops being written (whatever
/// reached the file stays) and keeps why, so the other copies and the stream
/// it is read from are unaffected and the stream reports it when joined.
pub(crate) struct Tee {
    /// Names the log in errors.
    label: &'static str,
    path: String,
    /// `None` once failed or finished.
    out: Option<Box<dyn Write + Send>>,
    error: Option<String>,
}

impl Tee {
    pub(crate) fn open(label: &'static str, path: String) -> Self {
        match File::create(&path) {
            Ok(f) => Self::new(label, path, Box::new(BufWriter::new(f))),
            Err(e) => Self {
                label,
                error: Some(format!("{label}: failed to create '{path}': {e}")),
                path,
                out: None,
            },
        }
    }

    fn new(label: &'static str, path: String, out: Box<dyn Write + Send>) -> Self {
        Self {
            label,
            path,
            out: Some(out),
            error: None,
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        let Some(out) = self.out.as_mut() else { return };
        if let Err(e) = out.write_all(bytes) {
            let error = format!("{}: write to '{}' failed: {e}", self.label, self.path);
            self.fail(error);
        }
    }

    /// Stop writing the copy, keeping why.
    fn fail(&mut self, error: String) {
        self.out = None;
        self.error.get_or_insert(error);
    }

    /// Start the copy over, empty, as bazel starts its own log over for a
    /// retried attempt. A copy that has failed stays failed.
    fn restart(&mut self) {
        if self.out.is_none() || self.finish().is_some() {
            return;
        }
        *self = Tee::open(self.label, std::mem::take(&mut self.path));
    }

    /// Flush the copy; returns why it is incomplete, if it is.
    fn finish(&mut self) -> Option<String> {
        if let Some(mut out) = self.out.take()
            && let Err(e) = out.flush()
        {
            self.error = Some(format!(
                "{}: flush of '{}' failed: {e}",
                self.label, self.path
            ));
        }
        self.error.clone()
    }
}

/// Wraps a `Read` source and tees every byte read to the [`Tee`]s, so they
/// capture the raw bytes without a second pass.
pub(crate) struct MultiTeeReader<R: Read> {
    pub(crate) inner: R,
    pub(crate) tees: Vec<Tee>,
}

impl<R: Read> Read for MultiTeeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        for tee in &mut self.tees {
            tee.write(&buf[..n]);
        }
        Ok(n)
    }
}

impl<R: Read> MultiTeeReader<R> {
    /// Start every copy over; see [`Tee::restart`].
    pub(crate) fn restart(&mut self) {
        self.tees.iter_mut().for_each(Tee::restart);
    }

    /// Start every copy over from `source`, the whole log, so the copies end
    /// as its file does whatever order its bytes were read in. `what` says
    /// what was being copied when a failure is reported.
    pub(crate) fn recopy(&mut self, source: io::Result<File>, what: &str) {
        if self.tees.is_empty() {
            return;
        }
        self.restart();
        let fail_all = |tees: &mut Vec<Tee>, e: io::Error| {
            for t in tees.iter_mut() {
                let error = format!("{}: {what} failed: {e}", t.label);
                t.fail(error);
            }
        };
        let mut file = match source {
            Ok(file) => file,
            Err(e) => return fail_all(&mut self.tees, e),
        };
        let mut buf = vec![0; 64 * 1024];
        loop {
            match file.read(&mut buf) {
                Ok(0) => return,
                Ok(n) => self.tees.iter_mut().for_each(|t| t.write(&buf[..n])),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return fail_all(&mut self.tees, e),
            }
        }
    }

    /// Finish every copy, failing with each failed copy's reason.
    pub(crate) fn finish(&mut self) -> io::Result<()> {
        let failures: Vec<String> = self.tees.iter_mut().filter_map(Tee::finish).collect();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(failures.join("; ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copy that cannot open costs only itself: the reader still passes every
    /// byte through, the good copy gets them all, and `finish` names the bad one.
    #[test]
    fn a_tee_that_cannot_open_fails_alone() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.zstd");
        let bad = dir.path().join("missing-dir").join("bad.zstd");
        let bytes = b"compact log bytes".to_vec();
        let mut tee = MultiTeeReader {
            inner: bytes.as_slice(),
            tees: [&bad, &good]
                .map(|p| Tee::open("ExecLog", p.to_str().unwrap().to_string()))
                .into(),
        };

        let mut read = vec![];
        tee.read_to_end(&mut read).unwrap();
        let err = tee.finish().unwrap_err().to_string();

        assert_eq!(read, bytes);
        assert_eq!(std::fs::read(&good).unwrap(), bytes);
        assert!(
            err.contains(bad.to_str().unwrap()) && err.contains("failed to create"),
            "{err}"
        );
    }

    /// A writer that accepts `budget` bytes, then fails every write or flush.
    struct Failing {
        budget: usize,
        fail_flush: bool,
    }

    impl Write for Failing {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf.len() > self.budget {
                return Err(io::Error::other("disk full"));
            }
            self.budget -= buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                Err(io::Error::other("disk full"))
            } else {
                Ok(())
            }
        }
    }

    fn failing(path: &str, budget: usize, fail_flush: bool) -> Tee {
        Tee::new(
            "ExecLog",
            path.to_owned(),
            Box::new(Failing { budget, fail_flush }),
        )
    }

    /// A copy whose write fails stops there; the reader still delivers every
    /// byte, and `finish` names it.
    #[test]
    fn a_tee_whose_write_fails_fails_alone() {
        let mut tee = MultiTeeReader {
            inner: &b"0123456789"[..],
            tees: vec![failing("full.zstd", 4, false)],
        };
        let mut buf = [0u8; 5];
        let mut read = vec![];
        loop {
            let n = tee.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            read.extend_from_slice(&buf[..n]);
        }
        assert_eq!(read, b"0123456789");
        let err = tee.finish().unwrap_err().to_string();
        assert!(err.contains("write to 'full.zstd' failed"), "{err}");
    }

    /// A copy that cannot be written again says why it was being copied, and
    /// one that had already failed keeps its own reason.
    #[test]
    fn a_failed_recopy_is_named_and_an_earlier_failure_kept() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.zstd");
        let mut failed = failing("full.zstd", 0, false);
        failed.write(b"x");
        let mut tee = MultiTeeReader {
            inner: &b""[..],
            tees: vec![
                Tee::open("ExecLog", good.to_str().unwrap().to_owned()),
                failed,
            ],
        };
        tee.recopy(Err(io::Error::other("gone")), "copying 'exec.log' again");
        let err = tee.finish().unwrap_err().to_string();
        assert!(
            err.contains("ExecLog: copying 'exec.log' again failed: gone")
                && err.contains("write to 'full.zstd' failed"),
            "{err}"
        );
    }

    /// A copy whose bytes all went through but whose flush fails is reported.
    #[test]
    fn a_tee_whose_flush_fails_is_reported() {
        let mut tee = MultiTeeReader {
            inner: &b"abc"[..],
            tees: vec![failing("full.zstd", 100, true)],
        };
        std::io::copy(&mut tee, &mut std::io::sink()).unwrap();
        let err = tee.finish().unwrap_err().to_string();
        assert!(err.contains("flush of 'full.zstd' failed"), "{err}");
    }

    /// Every failed copy is named, not only the first.
    #[test]
    fn every_failed_tee_is_reported() {
        let mut tee = MultiTeeReader {
            inner: &b"abc"[..],
            tees: vec![failing("one.zstd", 0, false), failing("two.zstd", 0, false)],
        };
        std::io::copy(&mut tee, &mut std::io::sink()).unwrap();
        let err = tee.finish().unwrap_err().to_string();
        assert!(
            err.contains("one.zstd") && err.contains("two.zstd"),
            "{err}"
        );
    }
}
