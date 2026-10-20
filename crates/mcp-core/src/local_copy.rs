//! Find the local `.fig` copy of a figma.com link in a few folders.
//!
//! A `.fig` file does not store its figma.com file key, so a link is matched by file name
//! ([`FileHint::match_stem`]): the copy Figma's "Save local copy" writes is named after the
//! file, as is the link's slug. Folders are searched one level deep. Every match is returned
//! with its strength; picking among them is [`crate::link::best_matches`], which never guesses
//! by date between two files of the same name.
use crate::link::{FileHint, Match};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
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

/// Every `.fig` file directly inside one of `dirs` whose stem `hint` matches, with how it
/// matches, in folder order. Folders that are missing or unreadable are skipped; a folder
/// listed twice is searched once.
pub fn local_copies(dirs: &[PathBuf], hint: &FileHint) -> Vec<(PathBuf, Match)> {
    let mut seen = Vec::new();
    let mut found = Vec::new();
    for dir in dirs {
        let key = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut here: Vec<(PathBuf, Match)> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_fig(path) && path.is_file())
            .filter_map(|path| {
                let rank = hint.match_stem(path.file_stem()?.to_str()?)?;
                Some((path, rank))
            })
            .collect();
        here.sort();
        found.extend(here);
    }
    found
}

fn is_fig(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("fig"))
}
