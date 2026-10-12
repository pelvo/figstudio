use std::{backtrace::Backtrace, fmt};

/// An invalid node ID, file key or link, or a failed document load.
#[derive(Debug)]
pub struct CoreError {
    kind: Kind,
    backtrace: Backtrace,
}

#[derive(Debug)]
enum Kind {
    InvalidNodeId,
    FileKey,
    Open(figstudio_fig::FigError),
    InvalidLink,
}

impl CoreError {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
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

    /// Whether this error describes an invalid node ID.
    pub fn is_invalid_node_id(&self) -> bool {
        matches!(self.kind, Kind::InvalidNodeId)
    }

    /// Whether this error describes an invalid link.
    pub fn is_invalid_link(&self) -> bool {
        matches!(self.kind, Kind::InvalidLink)
    }

    /// Whether loading a local document failed.
    pub fn is_open(&self) -> bool {
        matches!(self.kind, Kind::Open(_))
    }

    /// Return the captured diagnostic backtrace.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            Kind::InvalidNodeId => f.write_str("invalid node ID"),
            Kind::FileKey => {
                f.write_str("fileKey is missing, unknown, or ambiguous; use a listed full path")
            }
            Kind::Open(cause) => write!(f, "could not open local .fig file: {cause}"),
            Kind::InvalidLink => f.write_str("invalid link"),
        }
    }
}

impl std::error::Error for CoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            Kind::Open(cause) => Some(cause),
            _ => None,
        }
    }
}
