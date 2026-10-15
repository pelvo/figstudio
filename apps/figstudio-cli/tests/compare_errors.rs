//! `compare` failures print fixed text: no user value, member name or path is reflected.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};

static FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/structure.fig"
);

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_figstudio-cli"))
        .args(args)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("figstudio-compare-err-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// A zip whose first member fails its CRC, so the reader names that member.
fn corrupt_archive(name: &str) -> PathBuf {
    let path = scratch(name);
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("private/archive-member-marker.txt", options)
        .unwrap();
    zip.write_all(b"not a canvas").unwrap();
    zip.start_file("canvas.fig", options).unwrap();
    zip.write_all(b"junk").unwrap();
    zip.finish().unwrap();
    let mut bytes = fs::read(&path).unwrap();
    let at = bytes
        .windows(12)
        .position(|w| w == b"not a canvas")
        .unwrap();
    bytes[at] ^= 1;
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn unreadable_older_file_prints_fixed_text() {
    let bad = corrupt_archive("older.fig");
    let out = scratch("older-out");
    let output = cli(&[
        "compare",
        bad.to_str().unwrap(),
        FIXTURE,
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        "figstudio-cli: cannot read the older file\n"
    );
}

#[test]
fn unreadable_newer_file_prints_fixed_text() {
    let bad = corrupt_archive("newer.fig");
    let out = scratch("newer-out");
    let output = cli(&[
        "compare",
        FIXTURE,
        bad.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        "figstudio-cli: cannot read the newer file\n"
    );
}

#[test]
fn bad_arguments_never_echo_the_value() {
    let out = scratch("args-out");
    let out = out.to_str().unwrap();
    for (args, marker) in [
        (
            vec![
                "compare",
                "a.fig",
                "b.fig",
                "secret-extra-positional.fig",
                "-o",
                out,
            ],
            "secret-extra-positional",
        ),
        (
            vec![
                "compare",
                FIXTURE,
                FIXTURE,
                "-o",
                out,
                "--node",
                "junk-node-marker",
            ],
            "junk-node-marker",
        ),
        (
            vec![
                "compare",
                FIXTURE,
                FIXTURE,
                "-o",
                out,
                "--secret-flag-marker",
            ],
            "secret-flag-marker",
        ),
        (
            vec![
                "compare",
                FIXTURE,
                FIXTURE,
                "-o",
                out,
                "--scale",
                "secret-scale-marker",
            ],
            "secret-scale-marker",
        ),
    ] {
        let output = cli(&args);
        let err = stderr(&output);
        assert_eq!(output.status.code(), Some(2), "{err}");
        assert!(err.contains("USAGE:"), "{err}");
        assert!(!err.contains(marker), "stderr echoes {marker}: {err}");
    }
}

#[test]
fn help_still_prints_usage() {
    let output = cli(&["help"]);
    assert_eq!(output.status.code(), Some(0));
}
