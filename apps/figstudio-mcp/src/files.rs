use crate::error::McpError;
use figstudio_fig::Document;
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

struct FileEntry {
    key: String,
    path: PathBuf,
    document: Mutex<Option<Arc<Document>>>,
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

#[derive(Debug, Serialize)]
pub struct FileInfo {
    pub key: String,
    pub path: PathBuf,
    /// None until a tool first opens this file.
    pub pages: Option<usize>,
}

impl FileRegistry {
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
                    document: Mutex::new(None),
                })
                .collect(),
        ))
    }

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

    pub async fn open(&self, key: Option<&str>) -> Result<Arc<Document>, McpError> {
        let file = self.resolve(key)?;
        let mut cached = file.document.lock().await;
        if let Some(document) = &*cached {
            return Ok(document.clone());
        }
        let path = file.path.clone();
        let document = tokio::task::spawn_blocking(move || Document::open(path))
            .await
            .map_err(McpError::load_task)?
            .map_err(McpError::open)?;
        let document = Arc::new(document);
        *cached = Some(document.clone());
        Ok(document)
    }

    pub async fn list(&self) -> Vec<FileInfo> {
        let mut files = Vec::with_capacity(self.0.len());
        for file in self.0.iter() {
            files.push(FileInfo {
                key: file.key.clone(),
                path: file.path.clone(),
                pages: file
                    .document
                    .lock()
                    .await
                    .as_ref()
                    .map(|doc| doc.pages.len()),
            });
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
