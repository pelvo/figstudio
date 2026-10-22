//! `compare` writes a local artifact folder; its stdout line stays shareable.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use figstudio_fig::{Document, Node, Obj, Val};

static FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/structure.fig"
);

/// A modified copy of the public structure fixture: one page layer renamed.
/// All names below are the test's own, never a real file's.
fn modified_copy(dir: &Path) -> PathBuf {
    let mut doc = Document::open(FIXTURE).unwrap();
    let page = doc.pages[0];
    let mut nodes = doc.nodes.clone();
    let rect = nodes
        .iter_mut()
        .find(|n| n.kind == "FRAME" && n.parent == Some(page))
        .expect("fixture has a page frame to rename");
    rect.data
        .set("name", Val::Str("cli-compare-renamed-rect".into()));
    doc.set_nodes(nodes).unwrap();
    let out = dir.join("new.fig");
    doc.save(&out).unwrap();
    out
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("figstudio-compare-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_figstudio-cli"))
        .args(args)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn assert_success(output: Output) -> String {
    assert_success_full(output).0
}

/// Success with stdout and stderr; stderr must be empty on success.
fn assert_success_full(output: Output) -> (String, String) {
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stderr), "", "success prints nothing on stderr");
    (text(&output.stdout), text(&output.stderr))
}

/// A copy of the fixture whose page frame is filled opaque magenta: a pixel change, no rename.
fn color_copy(dir: &Path) -> PathBuf {
    let mut doc = Document::open(FIXTURE).unwrap();
    let page = doc.pages[0];
    let mut nodes = doc.nodes.clone();
    let frame = nodes
        .iter_mut()
        .find(|n| n.kind == "FRAME" && n.parent == Some(page))
        .expect("fixture has a page frame");
    let mut color = Obj::new();
    for (k, v) in [("r", 1.0), ("g", 0.0), ("b", 1.0), ("a", 1.0)] {
        color.insert(k, Val::Float(v));
    }
    let mut paint = Obj::new();
    paint.insert("type", Val::Enum("SOLID"));
    paint.insert("visible", Val::Bool(true));
    paint.insert("opacity", Val::Float(1.0));
    paint.insert("color", Val::Obj(color));
    frame
        .data
        .set("fillPaints", Val::Arr(vec![Val::Obj(paint)]));
    *frame = Node::from_data(frame.data.clone()).unwrap();
    doc.set_nodes(nodes).unwrap();
    let out = dir.join("color.fig");
    doc.save(&out).unwrap();
    out
}

/// Strings from the document that must never appear on stdout.
fn secrets(doc: &Document) -> Vec<String> {
    let mut out: Vec<String> = doc.nodes.iter().map(|n| n.name.clone()).collect();
    out.extend(doc.file_name.clone());
    out.extend(doc.images.keys().cloned());
    out.extend(
        doc.nodes
            .iter()
            .filter_map(|n| n.data.path(&["textData", "characters"]))
            .filter_map(|v| v.as_str().map(str::to_owned)),
    );
    out.retain(|s| s.chars().count() >= 4);
    out.sort();
    out.dedup();
    out
}

fn assert_png(path: &Path) {
    let image = image::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let (w, h) = image.into_rgba8().dimensions();
    assert!(w > 0 && h > 0, "{} is empty", path.display());
}

/// Every PNG path the report names, resolved against the artifact dir.
fn png_paths(report: &serde_json::Value, dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for page in report["pages"].as_array().expect("pages array") {
        let Some(render) = page["render"].as_object() else {
            continue;
        };
        for field in ["old_png", "new_png", "diff_png"] {
            if let Some(path) = render[field].as_str() {
                out.push(dir.join(path));
            }
        }
    }
    out
}

#[test]
fn identical_pair_reports_an_empty_diff_and_both_artifacts() {
    let dir = TempDir::new();
    let out = dir.file("report");
    let stdout = assert_success(cli(&[
        "compare",
        FIXTURE,
        FIXTURE,
        "-o",
        out.to_str().unwrap(),
    ]));
    assert_eq!(
        stdout,
        format!(
            "pages added 0, removed 0, changed 0, nodes added 0, removed 0, changed 0, moved 0\n{}/\n",
            out.display()
        )
    );
    let report = fs::read_to_string(out.join("report.json")).unwrap();
    assert!(report.contains("\"pages\":[]"), "{report}");
    assert!(fs::read(out.join("report.html")).is_ok());
}

#[test]
fn modified_pair_reports_counts_pngs_and_no_names() {
    let dir = TempDir::new();
    let new = modified_copy(&dir.0);
    let out = dir.file("report");
    let (stdout, stderr) = assert_success_full(cli(&[
        "compare",
        FIXTURE,
        new.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]));
    assert_eq!(
        stdout.lines().next().unwrap(),
        "pages added 0, removed 0, changed 1, nodes added 0, removed 0, changed 1, moved 0"
    );
    assert!(stdout.contains(out.display().to_string().as_str()));
    // Counts only: no layer names, file names or image keys on stdout.
    let mut forbidden = secrets(&Document::open(FIXTURE).unwrap());
    forbidden.extend(secrets(&Document::open(&new).unwrap()));
    forbidden.push("cli-compare-renamed-rect".into());
    for secret in forbidden {
        assert!(!stdout.contains(&secret), "stdout leaks {secret:?}");
        assert!(!stderr.contains(&secret), "stderr leaks {secret:?}");
    }
    let json = fs::read_to_string(out.join("report.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["summary"]["nodes_changed"], 1);
    // Every PNG the report names exists next to it and is a real image.
    let pngs = png_paths(&parsed, &out);
    assert_eq!(pngs.len(), 3, "{json}");
    for png in &pngs {
        assert_png(png);
    }
    let html = fs::read_to_string(out.join("report.html")).unwrap();
    // A rename is not a design change: the page is folded, and its pictures stay on disk.
    assert!(!html.contains("<section"), "{html}");
    assert!(html.contains("1 page with no visible change"), "{html}");
}

#[test]
fn no_render_and_json_only_flags_are_respected() {
    let dir = TempDir::new();
    let new = modified_copy(&dir.0);
    let out = dir.file("json-only");
    assert_success(cli(&[
        "compare",
        FIXTURE,
        new.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--json-only",
    ]));
    assert!(out.join("report.json").metadata().is_ok());
    assert!(out.join("report.html").metadata().is_err());
    assert!(
        out.join("render").metadata().is_err(),
        "no html means no pngs"
    );

    let out = dir.file("no-render");
    assert_success(cli(&[
        "compare",
        FIXTURE,
        new.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--no-render",
    ]));
    assert!(out.join("report.html").metadata().is_ok());
    let json = fs::read_to_string(out.join("report.json")).unwrap();
    assert!(
        !json.contains("render/"),
        "paths must be absent without renders: {json}"
    );
}

#[test]
fn scale_flag_still_writes_pngs() {
    let dir = TempDir::new();
    let new = modified_copy(&dir.0);
    let out = dir.file("scaled");
    assert_success(cli(&[
        "compare",
        FIXTURE,
        new.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--scale",
        "0.5",
    ]));
    let json = fs::read_to_string(out.join("report.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let pngs = png_paths(&parsed, &out);
    assert_eq!(pngs.len(), 3);
    for png in &pngs {
        assert_png(png);
    }
}

#[test]
fn usage_and_file_errors_match_existing_exit_codes() {
    let dir = TempDir::new();
    let out = dir.file("r");
    for args in [
        vec!["compare"],
        vec!["compare", FIXTURE],
        vec!["compare", FIXTURE, FIXTURE],
        vec!["compare", FIXTURE, FIXTURE, "-o"],
        vec![
            "compare",
            FIXTURE,
            FIXTURE,
            "-o",
            out.to_str().unwrap(),
            "--wat",
        ],
        vec![
            "compare",
            FIXTURE,
            FIXTURE,
            FIXTURE,
            "-o",
            out.to_str().unwrap(),
        ],
    ] {
        let output = cli(&args);
        let stderr = text(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert!(stderr.contains("USAGE:"), "{stderr}");
    }
    // A bad file is a runtime error (exit 1) with a one-line message.
    let truncated = dir.file("truncated.fig");
    let mut bytes = fs::read(FIXTURE).unwrap();
    bytes.truncate(bytes.len() / 2);
    fs::write(&truncated, bytes).unwrap();
    let output = cli(&[
        "compare",
        truncated.to_str().unwrap(),
        FIXTURE,
        "-o",
        out.to_str().unwrap(),
    ]);
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert_eq!(stderr.trim_end().lines().count(), 1, "{stderr}");
    assert!(stderr.starts_with("figstudio-cli: "), "{stderr}");
    // stdout stays empty on failure: no partial artifact summary.
    assert!(output.stdout.is_empty());
    assert!(!out.join("report.json").metadata().is_ok());
}

/// Pixels of a PNG as RGBA.
fn pixels(path: &Path) -> image::RgbaImage {
    image::open(path).unwrap().into_rgba8()
}

#[test]
fn recolor_pair_shows_the_change_in_the_pngs() {
    let dir = TempDir::new();
    let new = color_copy(&dir.0);
    let out = dir.file("report");
    assert_success(cli(&[
        "compare",
        FIXTURE,
        new.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]));
    let json = fs::read_to_string(out.join("report.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let render = &parsed["pages"][0]["render"];
    assert!(render["pixels_changed"].as_u64().unwrap() > 0, "{json}");
    let before = pixels(&out.join(render["old_png"].as_str().unwrap()));
    let after = pixels(&out.join(render["new_png"].as_str().unwrap()));
    let diff = pixels(&out.join(render["diff_png"].as_str().unwrap()));
    assert_eq!(before.dimensions(), after.dimensions());
    assert_eq!(before.dimensions(), diff.dimensions());
    assert!(
        before.pixels().zip(after.pixels()).any(|(a, b)| a != b),
        "before and after must differ"
    );
    assert!(
        after.pixels().any(|p| p.0 == [255, 0, 255, 255]),
        "the new fill is drawn"
    );
    assert!(
        diff.pixels().any(|p| p.0 == [255, 0, 0, 255]),
        "the diff marks changed pixels red"
    );
}
