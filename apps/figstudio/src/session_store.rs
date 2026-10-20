//! Private, atomic desktop workspace metadata, independent of eframe's shared preferences.
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use crate::session::Session;

#[cfg(not(target_arch = "wasm32"))]
const MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Default)]
pub(crate) struct Store {
    #[cfg(not(target_arch = "wasm32"))]
    path: Option<PathBuf>,
    problem: Option<String>,
    cleanup_eframe: bool,
    #[cfg(not(target_arch = "wasm32"))]
    legacy: Option<Session>,
    #[cfg(not(target_arch = "wasm32"))]
    warned_limit: bool,
}

impl Store {
    pub(crate) fn memory() -> Self {
        Self::default()
    }

    pub(crate) fn load() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(dir) = dirs::config_dir() {
            return Self::at(dir.join("figstudio").join("workspace.json"));
        }
        Self::memory()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn at(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            ..Self::default()
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn read(&mut self, storage: Option<&dyn eframe::Storage>) -> Session {
        self.cleanup_eframe = false;
        if let Some(path) = &self.path {
            match read_file(path) {
                Ok(Some(mut session)) => {
                    self.limit(&mut session);
                    self.cleanup_eframe = storage.is_some();
                    return session;
                }
                Ok(None) => {}
                Err(_) => {
                    self.problem = Some("Saved workspace is unreadable. The original file was kept; other settings still save. Workspace changes will not be saved until you repair the file and restart the app.".to_owned());
                    self.path = None;
                }
            }
        }
        #[derive(Default, serde::Deserialize)]
        #[serde(default)]
        struct Legacy {
            workspace: Session,
        }
        let legacy = storage.and_then(|s| eframe::get_value::<Legacy>(s, crate::UI_STATE_KEY));
        if legacy.is_none()
            && self.problem.is_none()
            && storage
                .and_then(|s| s.get_string(crate::UI_STATE_KEY))
                .is_some()
        {
            self.problem = Some("Previous workspace metadata could not be read.".to_owned());
        }
        let mut session = legacy.unwrap_or_default().workspace;
        self.legacy = (!session.is_empty()).then(|| session.clone());
        self.limit(&mut session);
        if !session.is_empty() {
            self.save(&session);
        }
        session
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn read(&mut self, _storage: Option<&dyn eframe::Storage>) -> Session {
        Session::default()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn limit(&mut self, session: &mut Session) {
        let skipped = session.limit();
        if skipped != 0 && !self.warned_limit {
            self.warned_limit = true;
            self.problem = Some(format!(
                "Workspace limit: {skipped} saved files were skipped beyond the {}-file limit.",
                crate::session::MAX_SAVED_FILES
            ));
        }
    }

    pub(crate) fn save(&mut self, session: &Session) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.path.is_none() {
                return;
            }
            let mut session = session.clone();
            self.limit(&mut session);
            let write = || -> std::io::Result<()> {
                let text = serde_json::to_string(&session).map_err(std::io::Error::other)?;
                if text.len() as u64 > MAX_BYTES {
                    return Err(std::io::Error::other("workspace metadata is too large"));
                }
                if let Some(path) = &self.path {
                    crate::spaces::write_private(path, text.as_bytes())?;
                }
                Ok(())
            };
            if write().is_err() {
                tracing::warn!("workspace not saved; the previous snapshot was kept");
                self.problem = Some(
                    "Workspace could not be saved; the previous snapshot was kept.".to_owned(),
                );
            } else if self.legacy.take().is_some() {
                self.cleanup_eframe = true;
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = session;
    }

    /// Preserve only the old copy until private persistence has committed it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn legacy_backup(&self) -> Session {
        self.legacy.clone().unwrap_or_default()
    }

    pub(crate) fn take_problem(&mut self) -> Option<String> {
        self.problem.take()
    }

    pub(crate) fn take_migration(&mut self) -> bool {
        std::mem::take(&mut self.cleanup_eframe)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_file(path: &Path) -> std::io::Result<Option<Session>> {
    use std::io::Read as _;
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !metadata.is_file() {
        return Err(std::io::Error::other("workspace is not a regular file"));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(std::io::Error::other("workspace metadata is too large"));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(std::io::Error::other)
}
