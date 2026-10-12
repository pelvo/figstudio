//! File helpers for the tools that write files; only with the `fs` feature.

/// Move a finished temporary file to `path` with `permissions`, or public read (0644 on unix)
/// for a new file, so the result does not depend on the process umask. Replaces a link at
/// `path` rather than following it.
pub fn persist_file(
    temp: tempfile::NamedTempFile,
    path: &std::path::Path,
    permissions: Option<std::fs::Permissions>,
) -> std::io::Result<()> {
    #[cfg(unix)]
    let permissions = permissions.or_else(|| {
        use std::os::unix::fs::PermissionsExt;
        Some(std::fs::Permissions::from_mode(0o644))
    });
    if let Some(permissions) = permissions {
        temp.as_file().set_permissions(permissions)?;
    }
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}
