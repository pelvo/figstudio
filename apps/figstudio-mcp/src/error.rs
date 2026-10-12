use figstudio_mcp_core::CoreError;
use std::{backtrace::Backtrace, fmt};

/// An invalid request, failed document load, or failed token extraction task.
#[derive(Debug)]
pub struct McpError {
    kind: Kind,
    backtrace: Backtrace,
}

#[derive(Debug)]
enum Kind {
    Core(CoreError),
    LoadTask(tokio::task::JoinError),
    TokenTask(tokio::task::JoinError),
}

impl McpError {
    pub(crate) fn file_key() -> Self {
        CoreError::file_key().into()
    }

    pub(crate) fn open(cause: figstudio_fig::FigError) -> Self {
        CoreError::open(cause).into()
    }

    pub(crate) fn load_task(cause: tokio::task::JoinError) -> Self {
        Self {
            kind: Kind::LoadTask(cause),
            backtrace: Backtrace::capture(),
        }
    }

    pub(crate) fn token_task(cause: tokio::task::JoinError) -> Self {
        Self {
            kind: Kind::TokenTask(cause),
            backtrace: Backtrace::capture(),
        }
    }

    /// Whether this error describes an invalid node ID.
    pub fn is_invalid_node_id(&self) -> bool {
        matches!(&self.kind, Kind::Core(error) if error.is_invalid_node_id())
    }

    /// Whether loading a local document failed.
    pub fn is_open(&self) -> bool {
        matches!(&self.kind, Kind::Core(error) if error.is_open())
    }

    /// Return the captured diagnostic backtrace.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            Kind::Core(error) => error.fmt(f),
            Kind::LoadTask(_) => f.write_str("document loading task failed"),
            Kind::TokenTask(_) => f.write_str("token extraction task failed"),
        }
    }
}

impl std::error::Error for McpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            Kind::Core(error) => std::error::Error::source(error),
            Kind::LoadTask(cause) | Kind::TokenTask(cause) => Some(cause),
        }
    }
}

impl From<CoreError> for McpError {
    fn from(error: CoreError) -> Self {
        Self {
            kind: Kind::Core(error),
            backtrace: Backtrace::capture(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::McpError;

    #[tokio::test]
    async fn token_join_failure_is_distinct_from_loading_failure() {
        let handle = tokio::spawn(std::future::pending::<()>());
        handle.abort();
        let error = McpError::token_task(handle.await.unwrap_err());
        assert_eq!(error.to_string(), "token extraction task failed");
        assert!(std::error::Error::source(&error).is_some());
    }
}
