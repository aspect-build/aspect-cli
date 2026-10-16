//! The error type of every fallible call into the JVM.

use std::fmt;

/// An exception the JVM threw, as it was when it was caught.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaException {
    /// The exception's class, e.g. `java.lang.IllegalArgumentException`.
    pub class: String,
    /// `getMessage()`, `None` when it is null.
    pub message: Option<String>,
    /// What `printStackTrace` prints: the exception, its frames and its causes.
    pub trace: String,
}

impl fmt::Display for JavaException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.trace)
    }
}

impl std::error::Error for JavaException {}

#[derive(Debug)]
enum Kind {
    Java(JavaException),
    Jni(jni::errors::Error),
    Message(String),
}

/// What went wrong in a call into the JVM.
///
/// A Java exception is [`Error::java`], and also the [`source`](std::error::Error::source)
/// of the error, so the full trace is available to error reporters. `Display`
/// shows the exception line and its first frame; context added with
/// [`Error::context`] comes first.
#[derive(Debug)]
pub struct Error {
    context: Vec<String>,
    kind: Kind,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// An error that is not an exception.
    pub fn msg(message: impl Into<String>) -> Error {
        Error {
            context: Vec::new(),
            kind: Kind::Message(message.into()),
        }
    }

    pub(crate) fn java(e: JavaException) -> Error {
        Error {
            context: Vec::new(),
            kind: Kind::Java(e),
        }
    }

    /// The exception the JVM threw, when this error is one.
    pub fn exception(&self) -> Option<&JavaException> {
        match &self.kind {
            Kind::Java(e) => Some(e),
            _ => None,
        }
    }

    /// Prefixes the error with what the caller was doing.
    #[must_use]
    pub fn context(mut self, context: impl fmt::Display) -> Error {
        self.context.insert(0, context.to_string());
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in &self.context {
            write!(f, "{c}: ")?;
        }
        match &self.kind {
            Kind::Java(e) => {
                let mut lines = e.trace.lines();
                let head = lines.next().unwrap_or(&e.class).trim();
                match lines.next() {
                    Some(frame) => write!(f, "{head}\n    {}", frame.trim()),
                    None => f.write_str(head),
                }
            }
            Kind::Jni(e) => write!(f, "{e}"),
            Kind::Message(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            Kind::Java(e) => Some(e),
            Kind::Jni(e) => Some(e),
            Kind::Message(_) => None,
        }
    }
}

impl From<jni::errors::Error> for Error {
    fn from(e: jni::errors::Error) -> Error {
        Error {
            context: Vec::new(),
            kind: Kind::Jni(e),
        }
    }
}
