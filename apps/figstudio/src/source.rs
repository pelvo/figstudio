//! Where a lazily opened document's images are read from: the file on disk (desktop) or the
//! browser `File` the user picked or dropped (read in slices, never whole).

use std::sync::Arc;

use figstudio_fig::archive::Entry;
use sha2::{Digest, Sha256};

/// Maximum prefix of a file hashed into [`doc_key_from_prefix`] (4 MiB).
pub const DOC_KEY_PREFIX_BYTES: usize = 4 * 1024 * 1024;

/// First 16 lowercase hex chars of SHA-256 over the 8-byte little-endian file size
/// followed by at most the first 4 MiB of the file (`docs/MULTI-DEVICE-SESSIONS-PLAN.md` §3.1).
pub fn doc_key_from_prefix(file_size: u64, head: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(file_size.to_le_bytes());
    let prefix = &head[..head.len().min(DOC_KEY_PREFIX_BYTES)];
    hasher.update(prefix);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(16);
    for b in &digest[..8] {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// Computes [`doc_key_from_prefix`] for a file on disk (`None` if the path cannot be read).
#[cfg(not(target_arch = "wasm32"))]
pub fn doc_key_from_path(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let to_read = usize::try_from(size.min(DOC_KEY_PREFIX_BYTES as u64)).ok()?;
    let mut buf = vec![0u8; to_read];
    file.read_exact(&mut buf).ok()?;
    Some(doc_key_from_prefix(size, &buf))
}

/// Reads archive entries of one open file. Blocking; call it on a worker thread.
pub trait ImageSource: Send + Sync {
    fn read(&self, entry: &Entry) -> anyhow::Result<Vec<u8>>;
}

pub type Source = Arc<dyn ImageSource>;

#[cfg(not(target_arch = "wasm32"))]
pub struct DiskSource {
    pub path: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl ImageSource for DiskSource {
    fn read(&self, entry: &Entry) -> anyhow::Result<Vec<u8>> {
        read_disk(&self.path, entry)
    }
}

/// The disk read behind [`DiskSource`], with each failing step named in its error.
#[cfg(not(target_arch = "wasm32"))]
fn read_disk(path: &std::path::Path, entry: &Entry) -> anyhow::Result<Vec<u8>> {
    use anyhow::Context;
    let mut file = std::fs::File::open(path).context("open the file")?;
    figstudio_fig::archive::read_entry(&mut file, entry).context("read an image")
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_keeps_its_io_cause() {
        let source = DiskSource {
            path: std::env::temp_dir().join("figstudio-no-such-file.fig"),
        };
        let entry = Entry {
            name: "images/ab".into(),
            method: 0,
            compressed_size: 1,
            size: 1,
            header_offset: 0,
        };
        let error = source.read(&entry).unwrap_err();
        assert!(
            error.chain().any(|c| c.is::<std::io::Error>()),
            "the io::Error was flattened: {error:?}"
        );
    }
}
