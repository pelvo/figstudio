//! Finding the local copy of a figma.com link by file name, without guessing between namesakes.
use figstudio_mcp_core::{
    link::{FileHint, Match, best_matches, parse_link},
    local_copy::{link_dirs_from, local_copies},
};
use std::{
    ffi::OsString,
    fs::File,
    path::{Path, PathBuf},
};

fn hint(url: &str) -> FileHint {
    parse_link(url).unwrap().file.unwrap()
}

fn touch(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    File::create(&path).unwrap();
    path
}

#[test]
fn figma_hints_rank_the_key_above_the_name_and_ignore_repeated_download_suffixes() {
    let link = hint("https://www.figma.com/design/AbC123/My-File-v2?node-id=1-2");
    for (stem, rank) in [
        ("AbC123", Match::Key),
        ("AbC123 (12)", Match::Key),
        ("My File v2", Match::Name),
        ("My-File-v2", Match::Name),
        ("My File v2 (1)", Match::Name),
    ] {
        assert_eq!(link.match_stem(stem), Some(rank), "{stem}");
    }
    for stem in [
        "My File",
        "My File v2 ()",
        "My File v2 (x)",
        "My File v2(1)",
        "abc123",
    ] {
        assert_eq!(link.match_stem(stem), None, "{stem}");
    }
    assert_eq!(link.name(), "My-File-v2");
}

#[test]
fn a_file_really_named_with_a_number_in_brackets_matches_its_own_slug() {
    let link = hint("https://www.figma.com/design/K/Design-(1)?node-id=1-2");
    assert_eq!(link.match_stem("Design (1)"), Some(Match::Name));
    assert_eq!(link.match_stem("Design (1) (2)"), Some(Match::Name));
    assert_eq!(link.match_stem("Design"), None);
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
    assert_eq!(link.match_stem("name (1)"), Some(Match::Key));
    assert!(!link.matches("name"));
}

#[test]
fn best_matches_keeps_every_tie_and_prefers_the_key() {
    assert_eq!(
        best_matches([("a", Match::Name), ("b", Match::Key), ("c", Match::Name)]),
        ["b"]
    );
    assert_eq!(
        best_matches([("a", Match::Name), ("c", Match::Name)]),
        ["a", "c"]
    );
    assert!(best_matches(Vec::<(u8, Match)>::new()).is_empty());
}

#[test]
fn local_copies_lists_every_match_in_every_folder_once() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let plain = touch(a.path(), "Design.fig");
    let upper = touch(a.path(), "Design (2).FIG");
    let again = touch(b.path(), "Design (1).fig");
    let keyed = touch(b.path(), "K.fig");
    touch(a.path(), "Other.fig");
    touch(a.path(), "Design.png");
    std::fs::create_dir(a.path().join("Design (3).fig")).unwrap();
    let dirs = vec![
        a.path().to_owned(),
        PathBuf::from("/no/such/dir"),
        b.path().to_owned(),
        a.path().to_owned(),
    ];
    let link = hint("https://www.figma.com/design/K/Design?node-id=1-2");
    let found = local_copies(&dirs, &link);
    assert_eq!(
        found,
        // Sorted by path within a folder: "Design (2)" comes before "Design.".
        [
            (upper.clone(), Match::Name),
            (plain.clone(), Match::Name),
            (again.clone(), Match::Name),
            (keyed.clone(), Match::Key),
        ]
    );
    // The key wins outright; without it, three namesakes tie and nobody guesses.
    assert_eq!(best_matches(found.clone()), [keyed]);
    let names = found.into_iter().filter(|(_, rank)| *rank == Match::Name);
    assert_eq!(best_matches(names), [upper, plain, again]);
}

#[test]
fn subfolders_are_not_searched() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    touch(&dir.path().join("sub"), "Design.fig");
    let link = hint("https://www.figma.com/design/K/Design?node-id=1-2");
    assert!(local_copies(&[dir.path().to_owned()], &link).is_empty());
    assert!(local_copies(&[], &link).is_empty());
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
