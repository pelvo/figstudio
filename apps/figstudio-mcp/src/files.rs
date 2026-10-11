use crate::error::McpError;
use figstudio_fig::Document;
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::OnceCell;

struct FileEntry {
    key: String,
    path: PathBuf,
    document: Arc<OnceCell<Arc<Document>>>,
}

/// Shared registry. Documents are opened on demand, once per registered file.
#[derive(Clone)]
pub struct FileRegistry(Arc<Vec<FileEntry>>);

impl std::fmt::Debug for FileRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileRegistry")
            .field("files", &self.0.len())
            .finish()
    }
}

/// Discovery information for a registered file.
#[derive(Debug, Serialize)]
pub struct FileInfo {
    /// Stable file stem used for selection.
    pub key: String,
    /// Registered path, usable to disambiguate duplicate stems.
    pub path: PathBuf,
    /// None until a tool first opens this file.
    pub pages: Option<usize>,
}

impl FileRegistry {
    /// Register paths without opening them.
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self(Arc::new(
            paths
                .into_iter()
                .map(|path| FileEntry {
                    key: path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    path,
                    document: Arc::new(OnceCell::new()),
                })
                .collect(),
        ))
    }

    /// Return registered file stems.
    pub fn keys(&self) -> Vec<String> {
        self.0.iter().map(|file| file.key.clone()).collect()
    }

    fn resolve(&self, key: Option<&str>) -> Result<&FileEntry, McpError> {
        match key {
            None if self.0.len() == 1 => Ok(&self.0[0]),
            Some(key) => {
                // Full paths disambiguate files with the same stem.
                if let Some(file) = self.0.iter().find(|file| {
                    file.path.as_os_str() == key
                        || std::path::absolute(&file.path).is_ok_and(|path| path.as_os_str() == key)
                }) {
                    return Ok(file);
                }
                let mut matches = self.0.iter().filter(|file| file.key == key);
                let file = matches.next().ok_or_else(McpError::file_key)?;
                if matches.next().is_some() {
                    return Err(McpError::file_key());
                }
                Ok(file)
            }
            _ => Err(McpError::file_key()),
        }
    }

    /// The registered stem of the file `key` selects.
    pub fn key(&self, key: Option<&str>) -> Result<String, McpError> {
        self.resolve(key).map(|file| file.key.clone())
    }

    /// Return the registered path, using the same selection rules as document loading.
    ///
    /// # Errors
    /// Returns an error for a missing or ambiguous file key.
    pub fn path(&self, key: Option<&str>) -> Result<PathBuf, McpError> {
        Ok(self.resolve(key)?.path.clone())
    }

    /// Open once per file; failures remain retryable even after cancellation.
    pub async fn open(&self, key: Option<&str>) -> Result<Arc<Document>, McpError> {
        let file = self.resolve(key)?;
        let cached = file.document.clone();
        let path = file.path.clone();
        // The initializer outlives a cancelled request and caches its completed decode.
        tokio::spawn(async move {
            cached
                .get_or_try_init(|| async move {
                    tokio::task::spawn_blocking(move || Document::open(path))
                        .await
                        .map_err(McpError::load_task)?
                        .map(Arc::new)
                        .map_err(McpError::open)
                })
                .await
                .cloned()
        })
        .await
        .map_err(McpError::load_task)?
    }

    /// List cached page counts without waiting for pending loads.
    pub async fn list(&self) -> Vec<FileInfo> {
        let mut files = Vec::with_capacity(self.0.len());
        for file in self.0.iter() {
            files.push(FileInfo {
                key: file.key.clone(),
                path: file.path.clone(),
                pages: file.document.get().map(|doc| doc.pages.len()),
            });
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_does_not_wait_for_a_pending_load() {
        let files = FileRegistry::new(vec![PathBuf::from("structure.fig")]);
        let cell = files.0[0].document.clone();
        let mut pending =
            Box::pin(cell.get_or_try_init(std::future::pending::<Result<Arc<Document>, McpError>>));
        assert!(futures_util::poll!(&mut pending).is_pending());
        let listed = tokio::time::timeout(std::time::Duration::from_millis(100), files.list())
            .await
            .expect("list blocked on loading");
        assert_eq!(listed[0].pages, None);
    }

    #[tokio::test]
    async fn cancelled_open_still_populates_the_cache() {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
        let files = FileRegistry::new(vec![fixture]);
        let cell = files.0[0].document.clone();
        let mut blocked =
            Box::pin(cell.get_or_try_init(std::future::pending::<Result<Arc<Document>, McpError>>));
        assert!(futures_util::poll!(&mut blocked).is_pending());
        let mut open = Box::pin(files.open(None));
        assert!(futures_util::poll!(&mut open).is_pending());
        drop(open);
        drop(blocked);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while files.list().await[0].pages.is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled request discarded its load");
        let cached = cell.get().unwrap();
        assert!(Arc::ptr_eq(cached, &files.open(None).await.unwrap()));
    }

    #[test]
    fn resolves_single_and_multiple_files() {
        let one = FileRegistry::new(vec![PathBuf::from("structure.fig")]);
        assert_eq!(one.resolve(None).unwrap().key, "structure");
        assert!(one.resolve(Some("unknown")).is_err());
        let two = FileRegistry::new(vec![
            PathBuf::from("structure.fig"),
            PathBuf::from("vector.fig"),
        ]);
        assert!(two.resolve(None).is_err());
        assert_eq!(
            two.resolve(Some("vector")).unwrap().path,
            PathBuf::from("vector.fig")
        );
        assert_eq!(two.resolve(Some("structure.fig")).unwrap().key, "structure");
        assert!(two.resolve(Some("unknown")).is_err());
        let absolute = std::path::absolute("vector.fig").unwrap();
        assert_eq!(two.resolve(absolute.to_str()).unwrap().key, "vector");
    }

    #[test]
    fn duplicate_stems_require_paths() {
        let files = FileRegistry::new(vec![
            PathBuf::from("a/example.fig"),
            PathBuf::from("b/example.fig"),
        ]);
        assert!(files.resolve(Some("example")).is_err());
        assert_eq!(
            files.resolve(Some("b/example.fig")).unwrap().path,
            PathBuf::from("b/example.fig")
        );
        assert!(FileRegistry::new(vec![]).resolve(None).is_err());
    }
}
