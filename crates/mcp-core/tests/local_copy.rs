//! Finding the local copy of a figma.com link by file name.
use figstudio_mcp_core::{
    link::{FileHint, parse_link},
    local_copy::{find_local_copy, link_dirs_from},
};
use std::{
    ffi::OsString,
    fs::File,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

fn hint(url: &str) -> FileHint {
    parse_link(url).unwrap().file.unwrap()
}

/// An empty file in `dir`, modified `age_secs` seconds before a fixed time.
fn touch(dir: &Path, name: &str, age_secs: u64) -> PathBuf {
    let path = dir.join(name);
    let file = File::create(&path).unwrap();
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
    file.set_modified(base - Duration::from_secs(age_secs))
        .unwrap();
    path
}

#[test]
fn figma_hints_match_the_key_the_slug_and_repeated_downloads() {
    let link = hint("https://www.figma.com/design/AbC123/My-File-v2?node-id=1-2");
    for stem in [
        "AbC123",
        "My File v2",
        "My-File-v2",
        "My File v2 (1)",
        "AbC123 (12)",
    ] {
        assert!(link.matches(stem), "{stem}");
    }
    for stem in [
        "My File",
        "My File v2 ()",
        "My File v2 (x)",
        "My File v2(1)",
        "abc123",
    ] {
        assert!(!link.matches(stem), "{stem}");
    }
    assert_eq!(link.name(), "My-File-v2");
}

#[test]
fn a_percent_encoded_unicode_slug_matches_the_decoded_name() {
    let url = "https://www.figma.com/design/K/%E6%97%A5%E6%9C%AC%E8%AA%9E%EF%BC%88%E7%89%88%EF%BC%89?node-id=1-2";
    let link = hint(url);
    assert!(link.matches("日本語（版）"));
    assert!(link.matches("日本語（版） (2)"));
    assert_eq!(link.name(), "日本語（版）");
}

#[test]
fn figstudio_hints_match_exactly() {
    let link = hint("figstudio://name%20(1)?node-id=1-2");
    assert!(link.matches("name (1)"));
    assert!(!link.matches("name"));
}

#[test]
fn the_newest_matching_fig_in_any_folder_wins() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    touch(a.path(), "Design.fig", 300);
    let newest = touch(b.path(), "Design (1).fig", 10);
    touch(a.path(), "Design (2).FIG", 100);
    // Newer, but not a match or not a .fig.
    touch(a.path(), "Other.fig", 0);
    touch(a.path(), "Design.png", 0);
    std::fs::create_dir(a.path().join("Design (3).fig")).unwrap();
    let dirs = vec![
        a.path().to_owned(),
        PathBuf::from("/no/such/dir"),
        b.path().to_owned(),
    ];
    let link = hint("https://www.figma.com/design/K/Design?node-id=1-2");
    assert_eq!(find_local_copy(&dirs, &link), Some(newest));
}

#[test]
fn subfolders_are_not_searched_and_no_match_is_none() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    touch(&dir.path().join("sub"), "Design.fig", 0);
    let link = hint("https://www.figma.com/design/K/Design?node-id=1-2");
    assert_eq!(find_local_copy(&[dir.path().to_owned()], &link), None);
    assert_eq!(find_local_copy(&[], &link), None);
}

#[test]
fn link_dirs_come_from_the_environment_else_downloads() {
    let home = Some(PathBuf::from("/home/u"));
    let list = std::env::join_paths(["/a", "/b"]).unwrap();
    assert_eq!(
        link_dirs_from(Some(list), home.clone()),
        [PathBuf::from("/a"), PathBuf::from("/b")]
    );
    for env in [None, Some(OsString::new())] {
        assert_eq!(
            link_dirs_from(env, home.clone()),
            [PathBuf::from("/home/u/Downloads")]
        );
    }
    assert!(link_dirs_from(None, None).is_empty());
}
