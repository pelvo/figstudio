use std::{backtrace::Backtrace, fmt};

/// An invalid node ID, unresolved file key, or failed document load.
#[derive(Debug)]
pub struct McpError {
    kind: Kind,
    backtrace: Backtrace,
}

#[derive(Debug)]
enum Kind {
    InvalidNodeId,
    FileKey,
    Open(figstudio_fig::FigError),
    LoadTask(tokio::task::JoinError),
}

impl McpError {
    pub(crate) fn invalid_node_id() -> Self {
        Self {
            kind: Kind::InvalidNodeId,
            backtrace: Backtrace::capture(),
        }
    }

    pub(crate) fn file_key() -> Self {
        Self {
            kind: Kind::FileKey,
            backtrace: Backtrace::capture(),
        }
    }

    pub(crate) fn open(cause: figstudio_fig::FigError) -> Self {
        Self {
            kind: Kind::Open(cause),
            backtrace: Backtrace::capture(),
        }
    }

    pub(crate) fn load_task(cause: tokio::task::JoinError) -> Self {
        Self {
            kind: Kind::LoadTask(cause),
            backtrace: Backtrace::capture(),
        }
    }

    /// Whether this error describes an invalid node ID.
    pub fn is_invalid_node_id(&self) -> bool {
        matches!(self.kind, Kind::InvalidNodeId)
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

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            Kind::InvalidNodeId => f.write_str("invalid node ID"),
            Kind::FileKey => {
                f.write_str("fileKey is missing, unknown, or ambiguous; use a listed full path")
            }
            Kind::Open(cause) => write!(f, "could not open local .fig file: {cause}"),
            Kind::LoadTask(_) => f.write_str("document loading task failed"),
        }
    }
}

impl std::error::Error for McpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            Kind::Open(cause) => Some(cause),
            Kind::LoadTask(cause) => Some(cause),
            _ => None,
        }
    }
}
