//! `figstudio-cli prototype` on a prototype built from the public fixture.
#[path = "../../../crates/render/tests/common/prototype.rs"]
mod proto;

use std::process::Command;

fn cli(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_figstudio-cli"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn lists_flows_screens_and_links_and_renders_the_screens() {
    let dir = tempfile::tempdir().unwrap();
    let file = proto::prototype_file(dir.path());
    let out = dir.path().join("screens");
    let (ok, stdout, stderr) = cli(&[
        "prototype",
        file.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--no-system-fonts",
    ]);
    assert!(ok, "{stderr}");
    for line in [
        "1. Start → 1:2 \"Groups\"",
        "screens (2):",
        "1:53 \"Group 2\"  on click → navigate 1:4 \"Symbols and images\"",
        "1:38 \"Component 2\"  on click → back",
        "on hover → overlay 1:2 \"Groups\" (not played)",
        "1 links not played",
    ] {
        assert!(stdout.contains(line), "missing {line:?} in:\n{stdout}");
    }
    for name in ["01-1-2.png", "02-1-4.png"] {
        let png = std::fs::read(out.join(name)).unwrap();
        assert!(png.starts_with(b"\x89PNG"), "{name}");
    }
}

#[test]
fn a_flow_is_picked_by_number_or_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = proto::prototype_file(dir.path());
    let file = file.to_str().unwrap();
    assert!(cli(&["prototype", file, "--flow", "Start"]).0);
    let (ok, _, stderr) = cli(&["prototype", file, "--flow", "2"]);
    assert!(
        !ok && stderr.contains("no flow 2; the file has 1"),
        "{stderr}"
    );
}

#[test]
fn a_file_without_flows_starts_at_the_first_frame() {
    let fixture = proto::fixture("structure.fig");
    let (ok, stdout, stderr) = cli(&["prototype", fixture.to_str().unwrap()]);
    assert!(ok, "{stderr}");
    assert!(
        stdout.starts_with("no flows; starting at the page's first frame"),
        "{stdout}"
    );
    assert!(
        stdout.contains("screens (1):\n  1:2 \"Groups\"\n0 links not played"),
        "{stdout}"
    );
}

#[test]
fn a_hidden_screen_is_not_played_and_does_not_stop_rendering() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = proto::prototype_doc();
    let mut nodes = doc.nodes.clone();
    for n in &mut nodes {
        if n.id == proto::SECOND {
            n.data.set("visible", figstudio_fig::Val::Bool(false));
        }
    }
    doc.set_nodes(nodes).unwrap();
    let file = dir.path().join("hidden.fig");
    doc.save(&file).unwrap();
    let out = dir.path().join("screens");
    let (ok, stdout, stderr) = cli(&[
        "prototype",
        file.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--no-system-fonts",
    ]);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("screens (1):"), "{stdout}");
    assert!(
        stdout.contains("navigate 1:4 \"Symbols and images\" (not played)"),
        "{stdout}"
    );
    assert!(stdout.contains("1 links not played"), "{stdout}");
}
