//! The file's shader folder: `FILE.fig.shaders/` next to the `.fig`, holding the zips Figma's
//! "View code" → "Download files" saves (or their unpacked folders). Read only.
use crate::shaders::ShaderSource;
use std::path::{Path, PathBuf};

/// Most sources read from one folder.
const MAX_SOURCES: usize = 200;
use crate::shaders::{MAX_SOURCE_FILE_BYTES as MAX_FILE_BYTES, MAX_SOURCE_FILES as MAX_FILES};

/// The shader folder of the `.fig` at `fig`.
pub fn folder(fig: &Path) -> PathBuf {
    let mut name = fig.file_name().unwrap_or_default().to_os_string();
    name.push(".shaders");
    fig.with_file_name(name)
}

/// Every source in the shader folder of `fig`; none when there is no folder. Unreadable,
/// oversized and non-text entries are left out.
pub fn load(fig: &Path) -> Vec<ShaderSource> {
    load_dir(&folder(fig))
}

/// Largest zip read.
const MAX_ZIP_BYTES: u64 = 20_000_000;

/// Every source in the folder `dir` (zips and unpacked folders); none when it cannot be read.
pub fn load_dir(dir: &Path) -> Vec<ShaderSource> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| Some(e.ok()?.path())).collect();
    paths.sort();
    paths
        .into_iter()
        .take(MAX_SOURCES)
        .filter_map(|path| {
            let label = path.file_stem()?.to_string_lossy().into_owned();
            // Links could point anywhere: only plain files and folders are read.
            let meta = std::fs::symlink_metadata(&path).ok()?;
            if meta.is_dir() {
                let files = from_folder(&path);
                (!files.is_empty()).then(|| ShaderSource::new(label, files))
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
                && meta.is_file()
                && meta.len() <= MAX_ZIP_BYTES
            {
                ShaderSource::from_zip(label, &std::fs::read(&path).ok()?)
            } else {
                None
            }
        })
        .collect()
}

fn text(bytes: Vec<u8>) -> Option<String> {
    String::from_utf8(bytes).ok()
}

fn from_folder(dir: &Path) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(String, String)> = entries
        .filter_map(|e| {
            let path = e.ok()?.path();
            let meta = std::fs::symlink_metadata(&path).ok()?;
            if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some((name, text(std::fs::read(&path).ok()?)?))
        })
        .take(MAX_FILES)
        .collect();
    files.sort();
    files
}
