//! Find the local `.fig` copy of a figma.com link in a few folders.
//!
//! A `.fig` file does not store its figma.com file key, so a link is matched by file name
//! ([`FileHint::matches`]): the copy Figma's "Save local copy" writes is named after the file,
//! as is the link's slug. Folders are searched one level deep; the newest match wins.
use crate::link::FileHint;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Environment variable with the folders to search, as a path list (`:`-separated on Unix).
pub const LINK_DIRS_ENV: &str = "FIGSTUDIO_LINK_DIRS";

/// The folders to search: `FIGSTUDIO_LINK_DIRS` when set and not empty, else `~/Downloads`.
pub fn link_dirs() -> Vec<PathBuf> {
    link_dirs_from(std::env::var_os(LINK_DIRS_ENV), std::env::home_dir())
}

/// [`link_dirs`] with the environment value and home folder given (for tests).
pub fn link_dirs_from(env: Option<OsString>, home: Option<PathBuf>) -> Vec<PathBuf> {
    let listed: Vec<PathBuf> = env
        .iter()
        .flat_map(std::env::split_paths)
        .filter(|dir| !dir.as_os_str().is_empty())
        .collect();
    if listed.is_empty() {
        home.map(|home| home.join("Downloads"))
            .into_iter()
            .collect()
    } else {
        listed
    }
}

/// The newest `.fig` file directly inside one of `dirs` whose stem `hint` matches. Folders
/// that are missing or unreadable are skipped.
pub fn find_local_copy(dirs: &[PathBuf], hint: &FileHint) -> Option<PathBuf> {
    dirs.iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_fig(path) && path.is_file())
        .filter(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| hint.matches(stem))
        })
        .max_by_key(|path| modified(path))
}

fn is_fig(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("fig"))
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}
