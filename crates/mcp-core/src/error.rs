use std::{backtrace::Backtrace, fmt};

/// An invalid node ID, file key or link, or a failed document load.
#[derive(Debug)]
pub struct CoreError {
    // Boxed so `Result<_, CoreError>` stays small (clippy::result_large_err).
    kind: Box<Kind>,
    backtrace: Backtrace,
}

#[derive(Debug)]
enum Kind {
    InvalidNodeId,
    FileKey,
    Open(figstudio_fig::FigError),
    InvalidLink,
    /// A request the file cannot satisfy: a missing node, a bad scale or rect.
    Input(String),
    /// A node or region that has nothing to draw.
    Render(String),
    /// A reply budget or size cap that was reached.
    Limit(String),
    Encode {
        format: &'static str,
        cause: Box<dyn std::error::Error + Send + Sync>,
    },
    Resolve {
        path: String,
        cause: figstudio_fig::resolve::ResolveError,
    },
}

impl CoreError {
    fn new(kind: Kind) -> Self {
        Self {
            kind: Box::new(kind),
            backtrace: Backtrace::capture(),
        }
    }

    /// A node ID that no accepted format describes.
    pub fn invalid_node_id() -> Self {
        Self::new(Kind::InvalidNodeId)
    }

    /// A file key that is missing, unknown or ambiguous.
    pub fn file_key() -> Self {
        Self::new(Kind::FileKey)
    }

    /// A local document that could not be opened.
    pub fn open(cause: figstudio_fig::FigError) -> Self {
        Self::new(Kind::Open(cause))
    }

    /// A link that is malformed or unsupported.
    pub fn invalid_link() -> Self {
        Self::new(Kind::InvalidLink)
    }

    /// A request the file cannot satisfy, with the message shown to the caller.
    pub fn input(message: impl Into<String>) -> Self {
        Self::new(Kind::Input(message.into()))
    }

    /// A node or region that has nothing to draw, with the message shown to the caller.
    pub fn render(message: impl Into<String>) -> Self {
        Self::new(Kind::Render(message.into()))
    }

    /// A reply budget or size cap that was reached, with the message shown to the caller.
    pub fn limit(message: impl Into<String>) -> Self {
        Self::new(Kind::Limit(message.into()))
    }

    /// An image that could not be encoded as `format` (for example "PNG").
    pub fn encode(
        format: &'static str,
        cause: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::new(Kind::Encode {
            format,
            cause: Box::new(cause),
        })
    }

    /// An instance path (shown as `path`) that could not be resolved.
    pub fn resolve(path: impl fmt::Display, cause: figstudio_fig::resolve::ResolveError) -> Self {
        Self::new(Kind::Resolve {
            path: path.to_string(),
            cause,
        })
    }

    /// Whether this error describes an invalid node ID.
    pub fn is_invalid_node_id(&self) -> bool {
        matches!(*self.kind, Kind::InvalidNodeId)
    }

    /// Whether this error describes an invalid link.
    pub fn is_invalid_link(&self) -> bool {
        matches!(*self.kind, Kind::InvalidLink)
    }

    /// Whether loading a local document failed.
    pub fn is_open(&self) -> bool {
        matches!(*self.kind, Kind::Open(_))
    }

    /// Return the captured diagnostic backtrace.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.kind {
            Kind::InvalidNodeId => f.write_str("invalid node ID"),
            Kind::FileKey => {
                f.write_str("fileKey is missing, unknown, or ambiguous; use a listed full path")
            }
            Kind::Open(cause) => write!(f, "could not open local .fig file: {cause}"),
            Kind::InvalidLink => f.write_str("invalid link"),
            Kind::Input(message) | Kind::Render(message) | Kind::Limit(message) => {
                f.write_str(message)
            }
            Kind::Encode { format, cause } => write!(f, "{format} encoding failed: {cause}"),
            Kind::Resolve { path, cause } => write!(f, "{path}: {cause}"),
        }
    }
}

impl std::error::Error for CoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &*self.kind {
            Kind::Open(cause) => Some(cause),
            Kind::Encode { cause, .. } => Some(cause.as_ref()),
            Kind::Resolve { cause, .. } => Some(cause),
            _ => None,
        }
    }
}
