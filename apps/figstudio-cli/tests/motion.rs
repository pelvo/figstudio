//! `render --node <id> -o out.<mp4|webm|gif|zip>`: an animated layer exported as a file.

use std::process::Command;

const MOTION: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/motion.fig"
);

fn run(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_figstudio-cli"))
        .args(args)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[test]
fn an_animated_layer_renders_as_a_gif_and_png_frames() {
    let dir = tempfile::tempdir().unwrap();
    let gif = dir.path().join("object.gif");
    let (ok, text) = run(&[
        "render",
        MOTION,
        "--node",
        "1:1106",
        "-o",
        gif.to_str().unwrap(),
        "--scale",
        "0.25",
        "--fps",
        "10",
    ]);
    assert!(ok, "{text}");
    assert!(text.contains("frames at 10 fps"), "{text}");
    assert!(std::fs::read(&gif).unwrap().starts_with(b"GIF89a"));
    let zip = dir.path().join("object.zip");
    let (ok, text) = run(&[
        "render",
        MOTION,
        "--node",
        "1:1106",
        "-o",
        zip.to_str().unwrap(),
        "--scale",
        "0.25",
    ]);
    assert!(ok, "{text}");
    assert!(std::fs::read(&zip).unwrap().starts_with(b"PK"));
}

#[test]
fn a_motion_export_needs_an_animated_node() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("x.mp4");
    let (ok, text) = run(&["render", MOTION, "-o", out.to_str().unwrap()]);
    assert!(!ok && text.contains("--node"), "{text}");
    let (ok, text) = run(&[
        "render",
        MOTION,
        "--node",
        "1:1106",
        "-o",
        out.to_str().unwrap(),
        "--fps",
        "0",
    ]);
    assert!(!ok && text.contains("fps"), "{text}");
}
