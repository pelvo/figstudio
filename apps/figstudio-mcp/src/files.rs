use crate::{error::McpError, live::LiveFiles, tools::rendering::Rendering};
use figstudio_fig::{Document, LAZY_ABOVE};
use figstudio_render::images::Images;
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::OnceCell;

/// Decoded-image bytes each registered file may keep between calls.
pub(crate) const IMAGE_CACHE_BYTES: usize = 256 * 1024 * 1024;

struct FileEntry {
    key: String,
    path: PathBuf,
    document: Arc<OnceCell<Arc<Document>>>,
    /// Where this file's renders read lazy images, and their shared cache.
    rendering: Rendering,
}

/// Shared registry. Documents are opened on demand, once per registered file: eagerly, or
/// lazily (images read when a render or `download_assets` needs them) above a size.
#[derive(Clone)]
pub struct FileRegistry {
    files: Arc<Vec<FileEntry>>,
    lazy_above: u64,
}

impl std::fmt::Debug for FileRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileRegistry")
            .field("files", &self.files.len())
            .field("lazy_above", &self.lazy_above)
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
    /// Files bigger than [`LAZY_ABOVE`] open lazily.
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self {
            files: Arc::new(
                paths
                    .into_iter()
                    .map(|path| FileEntry {
                        key: stem(&path),
                        rendering: Rendering::new(path.clone(), Images::new(IMAGE_CACHE_BYTES)),
                        path,
                        document: Arc::new(OnceCell::new()),
                    })
                    .collect(),
            ),
            lazy_above: LAZY_ABOVE,
        }
    }

    /// Open files bigger than `bytes` lazily instead (`0`: every archive, for tests).
    /// Applies to documents not opened yet.
    pub fn with_lazy_above(mut self, bytes: u64) -> Self {
        self.lazy_above = bytes;
        self
    }

    /// Return registered file stems.
    pub fn keys(&self) -> Vec<String> {
        self.files.iter().map(|file| file.key.clone()).collect()
    }

    fn resolve(&self, key: Option<&str>) -> Result<&FileEntry, McpError> {
        match key {
            None if self.files.len() == 1 => Ok(&self.files[0]),
            Some(key) => find(&self.files, key, |file| &file.path),
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

    /// The rendering of the entry whose registered path is `path` (the value
    /// `open_picked` returned), so a duplicate-stem pair never re-resolves by
    /// stem.
    ///
    /// # Errors
    /// Returns an error when `path` names no registered file.
    pub fn rendering_for_path(
        &self,
        path: &Path,
        system_fonts: bool,
    ) -> Result<Rendering, McpError> {
        let entry = self
            .files
            .iter()
            .find(|file| file.path.as_path() == path)
            .ok_or_else(McpError::file_key)?;
        let mut rendering = entry.rendering.clone();
        rendering.system_fonts = system_fonts;
        Ok(rendering)
    }

    /// How to render the file `key` selects: its path for lazily opened images and its
    /// shared image cache, with `system_fonts`.
    ///
    /// # Errors
    /// Returns an error for a missing or ambiguous file key.
    pub fn rendering(&self, key: Option<&str>, system_fonts: bool) -> Result<Rendering, McpError> {
        let mut rendering = self.resolve(key)?.rendering.clone();
        rendering.system_fonts = system_fonts;
        Ok(rendering)
    }

    /// Open the entry `file` (already resolved, so duplicate stems stay apart).
    async fn open_entry(file: &FileEntry, lazy_above: u64) -> Result<Arc<Document>, McpError> {
        let cached = file.document.clone();
        let path = file.path.clone();
        let images = file.rendering.images.clone();
        // The initializer outlives a cancelled request and caches its completed decode.
        tokio::spawn(async move {
            cached
                .get_or_try_init(|| async move {
                    tokio::task::spawn_blocking(move || {
                        let doc = Document::open_auto_above(&path, lazy_above)?;
                        if let Some(fills) = figstudio_shader::Fills::for_file(&path) {
                            images.set_shaders(doc.instance, fills);
                        }
                        Ok::<_, figstudio_fig::FigError>(doc)
                    })
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

    /// Open once per file; failures remain retryable even after cancellation.
    pub async fn open(&self, key: Option<&str>) -> Result<Arc<Document>, McpError> {
        Self::open_entry(self.resolve(key)?, self.lazy_above).await
    }

    /// Open, key and path from one resolution, so a duplicate-stem pair can
    /// never be re-resolved to the other entry between calls.
    pub async fn open_picked(
        &self,
        key: Option<&str>,
    ) -> Result<(Arc<Document>, String, PathBuf), McpError> {
        let file = self.resolve(key)?;
        let doc = Self::open_entry(file, self.lazy_above).await?;
        Ok((doc, file.key.clone(), file.path.clone()))
    }

    /// List cached page counts without waiting for pending loads.
    pub async fn list(&self) -> Vec<FileInfo> {
        let mut files = Vec::with_capacity(self.files.len());
        for file in self.files.iter() {
            files.push(FileInfo {
                key: file.key.clone(),
                path: file.path.clone(),
                pages: file.document.get().map(|doc| doc.pages.len()),
            });
        }
        files
    }
}

/// A file's key: its stem.
pub(crate) fn stem(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// The entry `key` names: one whose path is `key` (as given or absolute), else the only entry
/// whose stem is `key`. Full paths disambiguate files with the same stem.
pub(crate) fn find<'a, T>(
    entries: &'a [T],
    key: &str,
    path: impl Fn(&T) -> &Path,
) -> Result<&'a T, McpError> {
    if let Some(entry) = entries.iter().find(|entry| {
        let path = path(entry);
        path.as_os_str() == key
            || std::path::absolute(path).is_ok_and(|path| path.as_os_str() == key)
    }) {
        return Ok(entry);
    }
    let mut matches = entries.iter().filter(|entry| stem(path(entry)) == key);
    let entry = matches.next().ok_or_else(McpError::file_key)?;
    if matches.next().is_some() {
        return Err(McpError::file_key());
    }
    Ok(entry)
}

/// Where the tools find documents: registered files (standalone) or the app's open tabs.
#[derive(Clone, Debug)]
pub enum Files {
    /// Files named on the command line, opened on demand.
    Registry(FileRegistry),
    /// The tabs the app publishes; a missing `fileKey` means the active tab.
    Live(Arc<LiveFiles>),
}

impl Files {
    /// File keys: registered stems, or the open tabs' keys.
    pub fn keys(&self) -> Vec<String> {
        match self {
            Self::Registry(files) => files.keys(),
            Self::Live(live) => live.keys(),
        }
    }

    /// The key of the file `key` selects.
    ///
    /// # Errors
    /// Returns an error for a missing, unknown or ambiguous file key.
    pub fn key(&self, key: Option<&str>) -> Result<String, McpError> {
        match self {
            Self::Registry(files) => files.key(key),
            Self::Live(live) => live.key(key),
        }
    }

    /// The path of the file `key` selects.
    ///
    /// # Errors
    /// Returns an error for a missing, unknown or ambiguous file key.
    pub fn path(&self, key: Option<&str>) -> Result<PathBuf, McpError> {
        match self {
            Self::Registry(files) => files.path(key),
            Self::Live(live) => live.path(key),
        }
    }

    /// The rendering of the entry whose registered path is `path` (the value
    /// `open_picked` returned), so a duplicate-stem pair never re-resolves
    /// by stem.
    ///
    /// # Errors
    /// Returns an error for an unknown or ambiguous file key (standalone).
    pub fn rendering_picked(
        &self,
        path: &Path,
        doc: &Document,
        system_fonts: bool,
    ) -> Result<Rendering, McpError> {
        match self {
            Self::Registry(files) => files.rendering_for_path(path, system_fonts),
            Self::Live(live) => Ok(live.rendering(doc, system_fonts)),
        }
        .map(|mut rendering| {
            rendering.system_fonts = system_fonts;
            rendering
        })
    }

    /// How to render `doc`, which [`Files::open`] returned for `key`.
    ///
    /// # Errors
    /// Returns an error for a missing, unknown or ambiguous file key (standalone).
    pub fn rendering(
        &self,
        key: Option<&str>,
        doc: &Document,
        system_fonts: bool,
    ) -> Result<Rendering, McpError> {
        match self {
            Self::Registry(files) => files.rendering(key, system_fonts),
            Self::Live(live) => Ok(live.rendering(doc, system_fonts)),
        }
    }

    /// The document `key` selects. Live documents are the app's own, shared by `Arc`.
    ///
    /// # Errors
    /// Returns an error for a bad file key or a file that does not decode.
    pub async fn open(&self, key: Option<&str>) -> Result<Arc<Document>, McpError> {
        match self {
            Self::Registry(files) => files.open(key).await,
            Self::Live(live) => live.open(key),
        }
    }

    /// The document `key` selects with its file key and path, all from one look at the
    /// app's tabs: a tab swapped between separate `open`/`key`/`path` calls could mix files.
    ///
    /// # Errors
    /// Returns an error for a bad file key or a file that does not decode.
    pub async fn open_picked(
        &self,
        key: Option<&str>,
    ) -> Result<(Arc<Document>, String, PathBuf), McpError> {
        match self {
            Self::Registry(files) => files.open_picked(key).await,
            Self::Live(live) => {
                let picked = live.pick(key)?;
                let tab = picked.tab();
                Ok((tab.doc.clone(), picked.key.clone(), tab.path.clone()))
            }
        }
    }

    /// The files with their page counts (`None` until opened, standalone).
    pub async fn list(&self) -> Vec<FileInfo> {
        match self {
            Self::Registry(files) => files.list().await,
            Self::Live(live) => live.list(),
        }
    }
}

#[cfg(test)]
mod tests;
