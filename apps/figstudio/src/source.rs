//! Where a lazily opened document's images are read from: the file on disk (desktop) or the
//! browser `File` the user picked or dropped (read in slices, never whole).

use std::sync::Arc;

use figstudio_fig::archive::Entry;

/// Reads archive entries of one open file. Blocking; call it on a worker thread.
pub trait ImageSource: Send + Sync {
    fn read(&self, entry: &Entry) -> Result<Vec<u8>, String>;
}

pub type Source = Arc<dyn ImageSource>;

#[cfg(not(target_arch = "wasm32"))]
pub struct DiskSource {
    pub path: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl ImageSource for DiskSource {
    fn read(&self, entry: &Entry) -> Result<Vec<u8>, String> {
        let mut file = std::fs::File::open(&self.path).map_err(|e| e.to_string())?;
        figstudio_fig::archive::read_entry(&mut file, entry).map_err(|e| e.0)
    }
}

/// Opens `path`: lazily (with a source for its images) when it is a big archive.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_path(
    path: &std::path::Path,
) -> Result<(figstudio_fig::Document, Option<Source>), figstudio_fig::FigError> {
    let doc = figstudio_fig::Document::open_auto(path)?;
    let source: Option<Source> = (!doc.lazy_images.is_empty()).then(|| {
        Arc::new(DiskSource {
            path: path.to_path_buf(),
        }) as Source
    });
    Ok((doc, source))
}
