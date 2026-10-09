//! `doctor` output is meant to be pasted publicly, so it must not contain document content.

use std::path::PathBuf;
use std::process::Command;

use figstudio_fig::{Document, Val};
use std::io::Write as _;

const FIXTURES: [&str; 7] = [
    "broken_images",
    "corners",
    "noise_and_grain",
    "sections",
    "stacks_wrap",
    "structure",
    "vector",
];

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(format!("{name}.fig"))
}

/// Strings from the document that must never appear in the report.
fn secrets(doc: &Document, path: &std::path::Path) -> Vec<String> {
    let mut out = vec![path.display().to_string()];
    out.extend(path.file_stem().map(|s| s.to_string_lossy().to_string()));
    out.extend(doc.file_name.clone());
    out.extend(doc.images.keys().cloned());
    if let Ok(file) = std::fs::File::open(path)
        && let Ok(mut zip) = zip::ZipArchive::new(file)
    {
        out.extend(
            (0..zip.len())
                .filter_map(|i| zip.by_index(i).ok().map(|entry| entry.name().to_owned())),
        );
    }
    for n in &doc.nodes {
        out.push(n.name.clone());
        if let Some(t) = n
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str)
        {
            out.push(t.to_string());
            // Single words too, but only long ones; short words ("with") are the report's own.
            out.extend(
                t.split_whitespace()
                    .filter(|w| w.chars().count() >= 7)
                    .map(str::to_string),
            );
        }
        if let Some(f) = n.data.path(&["fontName", "family"]).and_then(Val::as_str) {
            out.push(f.to_string());
        }
        if let Some(u) = n.data.path(&["editInfo", "userId"]).and_then(Val::as_str) {
            out.push(u.to_string());
        }
    }
    // Very short strings ("A", "1") match by accident and carry no information.
    out.retain(|s| s.chars().count() >= 4);
    out.sort();
    out.dedup();
    out
}

/// `needle` occurs in `hay` with no letter or digit right before or after it, so a layer
/// called "Text" does not match the schema field name `derivedTextData`.
fn contains_word(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + needle.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

#[test]
fn doctor_output_contains_no_document_content() {
    let exe = env!("CARGO_BIN_EXE_figstudio-cli");
    let mut checked = 0;
    for name in FIXTURES {
        let path = fixture(name);
        let doc = Document::open(&path).unwrap();
        let out = Command::new(exe).arg("doctor").arg(&path).output().unwrap();
        assert!(
            out.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report = String::from_utf8(out.stdout).unwrap();
        assert!(
            report.contains("reads back identical: yes"),
            "{name}:\n{report}"
        );
        for s in secrets(&doc, &path) {
            assert!(!contains_word(&report, &s), "{name}: report contains {s:?}");
            checked += 1;
        }
    }
    assert!(checked > 50, "only {checked} strings checked");
}

#[test]
fn word_matching() {
    assert!(contains_word("a Text b", "Text"));
    assert!(!contains_word("derivedTextData 1", "Text"));
    assert!(contains_word(
        "x: Horizontal wrap top left",
        "Horizontal wrap top left"
    ));
}

#[test]
fn doctor_errors_do_not_echo_the_path() {
    let exe = env!("CARGO_BIN_EXE_figstudio-cli");
    let dir = std::env::temp_dir().join("figstudio-doctor-private-name");
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("secret-client-project.fig");
    std::fs::write(&bad, b"PK\x03\x04 definitely not a zip").unwrap();
    let named_member = dir.join("secret-member-project.fig");
    let file = std::fs::File::create(&named_member).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(
        "private-member-name.txt",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"not a canvas").unwrap();
    zip.finish().unwrap();
    let mut archive = std::fs::read(&named_member).unwrap();
    let member = b"private-member-name.txt";
    let member_at = archive
        .windows(member.len())
        .position(|w| w == member)
        .unwrap();
    archive[member_at + member.len()] ^= 1; // Invalid CRC makes the reader report this name.
    std::fs::write(&named_member, archive).unwrap();

    for path in [
        bad.clone(),
        dir.join("missing-secret-file.fig"),
        named_member,
    ] {
        let out = Command::new(exe).arg("doctor").arg(&path).output().unwrap();
        assert!(!out.status.success());
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!all.contains("secret"), "{all}");
        assert!(!all.contains("private-member-name"), "{all}");
    }
}
