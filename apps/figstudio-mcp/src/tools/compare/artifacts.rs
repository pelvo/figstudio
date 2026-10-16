use crate::tools::design::Assets;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

/// Best-effort retention; never follow symlinks or remove unrelated entries.
pub(super) fn sweep_old_artifacts(root: &Path, now: SystemTime, max_age: Duration) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("compare-") {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if metadata.is_dir()
            && metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > max_age)
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Fresh request-isolated directory, retained for clients to read after return.
pub(super) fn artifact_dir(assets: &Assets) -> std::io::Result<PathBuf> {
    let root = match assets {
        Assets::Dir(dir) => dir.join("compare"),
        Assets::Inline => std::env::temp_dir().join("figstudio-compare"),
    };
    if fs::symlink_metadata(&root).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(std::io::Error::other("artifact directory unavailable"));
    }
    // This preflight rejects planted root symlinks; concurrent replacement is out of scope.
    fs::create_dir_all(&root)?;
    sweep_old_artifacts(&root, SystemTime::now(), Duration::from_secs(24 * 60 * 60));
    Ok(tempfile::Builder::new()
        .prefix("compare-")
        .tempdir_in(&root)?
        .keep())
}
