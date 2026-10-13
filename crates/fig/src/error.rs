//! The error type of the decoder and writer.

use std::backtrace::Backtrace;
use std::error::Error as StdError;
use std::fmt;

/// A `.fig` file that could not be read, decoded, written or saved.
///
/// [`Error::source`](std::error::Error::source) returns the underlying cause (an I/O, archive
/// or codec error) when there is one. Messages never contain node names, text or colours.
#[derive(Debug)]
pub struct FigError(Box<Inner>);

// Boxed so `Result<_, FigError>` stays small (clippy::result_large_err, also for wrappers).
#[derive(Debug)]
struct Inner {
    kind: Kind,
    backtrace: Backtrace,
}

#[derive(Debug, thiserror::Error)]
enum Kind {
    #[error("{what}: {source}")]
    Io {
        what: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{what}: {source}")]
    Archive {
        what: &'static str,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("{what}: {source}")]
    Codec {
        what: &'static str,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
    /// A size limit was hit.
    #[error("{0}")]
    Limit(String),
    /// The data is not a valid `.fig` (or not valid to write).
    #[error("{0}")]
    Malformed(String),
}

impl FigError {
    fn new(kind: Kind) -> Self {
        Self(Box::new(Inner {
            kind,
            backtrace: Backtrace::capture(),
        }))
    }

    /// The data is not a valid `.fig`, or a document is not valid to write.
    pub fn malformed(message: impl Into<String>) -> Self {
        Self::new(Kind::Malformed(message.into()))
    }

    /// A size limit was hit.
    pub fn limit(message: impl Into<String>) -> Self {
        Self::new(Kind::Limit(message.into()))
    }

    /// An I/O failure; `what` is shown before the cause.
    pub fn io(what: impl Into<String>, source: std::io::Error) -> Self {
        Self::new(Kind::Io {
            what: what.into(),
            source,
        })
    }

    /// A zip archive failure; `what` is shown before the cause.
    pub fn archive(what: &'static str, source: zip::result::ZipError) -> Self {
        Self::new(Kind::Archive { what, source })
    }

    /// A compression codec failure; `what` is shown before the cause.
    pub fn codec(what: &'static str, source: impl StdError + Send + Sync + 'static) -> Self {
        Self::new(Kind::Codec {
            what,
            source: Box::new(source),
        })
    }

    /// Whether a size limit was hit.
    pub fn is_limit(&self) -> bool {
        matches!(self.0.kind, Kind::Limit(_))
    }

    /// The captured diagnostic backtrace.
    pub fn backtrace(&self) -> &Backtrace {
        &self.0.backtrace
    }
}

impl fmt::Display for FigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.kind, f)
    }
}

impl StdError for FigError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.kind.source()
    }
}

/// An error and its causes on one line, `outer: inner: root`. A cause is left out when the
/// message before it already ends with it: [`FigError`] (and errors that wrap it) print their
/// cause and also return it from `source()`, so `{:#}` would show it twice.
pub fn one_line(error: &(dyn StdError + 'static)) -> String {
    let mut line = error.to_string();
    let mut next = error.source();
    while let Some(cause) = next {
        let text = cause.to_string();
        if !line.ends_with(text.as_str()) {
            line.push_str(": ");
            line.push_str(&text);
        }
        next = cause.source();
    }
    line
}
