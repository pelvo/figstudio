use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use figstudio_fig::{Document, Val};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/structure.fig"
);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("figstudio-cli-{}-{id}", std::process::id()));
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
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    text(&output.stdout)
}

fn assert_usage_error(args: &[&str], expected: &str) {
    let output = cli(args);
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains(expected), "{stderr}");
    assert!(stderr.contains("USAGE:"), "{stderr}");
}

#[test]
fn unknown_command_is_usage_error_before_opening_file() {
    for args in [
        vec!["not-a-command"],
        vec!["not-a-command", "target/rustcli/missing.fig"],
    ] {
        assert_usage_error(&args, "unknown command not-a-command");
    }
}

#[test]
fn help_and_usage_errors_have_expected_exit_codes() {
    for help in ["--help", "-h", "help"] {
        assert!(assert_success(cli(&[help])).contains("USAGE:"));
    }
    assert_usage_error(&[], "missing command");
    assert_usage_error(&["unknown"], "unknown command unknown");
    assert_usage_error(&["info", FIXTURE, "--wat"], "unknown option --wat");
    for (flag, message) in [
        ("--page", "--page needs a value"),
        ("--node", "--node needs a value"),
        ("--scale", "--scale needs a value"),
        ("--depth", "--depth needs a value"),
        ("--background", "--background needs a value"),
        ("--copies", "--copies needs a value"),
        ("--pages", "--pages needs a value"),
        ("-o", "-o needs a value"),
    ] {
        assert_usage_error(&["info", FIXTURE, flag], message);
    }
    assert_usage_error(
        &["info", FIXTURE, "--node", "bad"],
        "node id must look like 12:34",
    );
    assert_usage_error(
        &["info", FIXTURE, "--depth", "bad"],
        "--depth must be a number",
    );
    assert_usage_error(
        &["info", FIXTURE, "--scale", "bad"],
        "--scale must be a number",
    );
    assert_usage_error(&["info", FIXTURE, "--scale", "33"], "between 0.01 and 32");
    for flag in ["--copies", "--pages"] {
        assert_usage_error(
            &["synth", FIXTURE, "-o", "out.fig", flag, "0"],
            "between 1 and 1000000",
        );
    }
    assert_usage_error(&["info", FIXTURE, "extra"], "unexpected argument extra");
    assert_usage_error(&["doctor"], "missing .fig file");
    let doctor = cli(&["doctor", FIXTURE]);
    assert_eq!(doctor.status.code(), Some(0), "{}", text(&doctor.stderr));
    assert!(text(&doctor.stdout).contains("reads back identical: yes"));
}

#[test]
fn output_commands_require_an_output_path_before_opening_input() {
    for (command, message) in [
        ("thumbnail", "thumbnail needs -o"),
        ("resave", "resave needs -o"),
        ("synth", "synth needs -o"),
    ] {
        assert_usage_error(&[command, "target/rustcli/missing.fig"], message);
    }
}

#[test]
fn valid_command_with_missing_or_malformed_input_is_runtime_error() {
    let dir = TempDir::new();
    let malformed = dir.file("broken.fig");
    fs::write(&malformed, b"not a fig archive").unwrap();
    for file in [dir.file("missing.fig"), malformed] {
        let output = cli(&["info", file.to_str().unwrap()]);
        assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
        assert!(!text(&output.stderr).contains("USAGE:"));
    }
}

#[test]
fn info_and_tree_report_document_structure() {
    let info = assert_success(cli(&["info", FIXTURE]));
    assert!(info.contains("format:   fig-kiwi v48"), "{info}");
    assert!(info.contains("nodes:    26"), "{info}");
    assert!(info.contains("1. Page 1"), "{info}");

    let tree = assert_success(cli(&["tree", FIXTURE, "--depth", "1"]));
    assert!(tree.contains("CANVAS \"Page 1\""), "{tree}");
    assert!(tree.contains("… "), "{tree}");
}

#[test]
fn tree_walk_is_bounded_by_requested_depth_on_deep_documents() {
    let dir = TempDir::new();
    let deep = dir.file("deep.fig");
    let mut doc = Document::open(FIXTURE).unwrap();
    let page = doc.pages[0];
    let template = doc
        .nodes
        .iter()
        .find(|node| node.kind == "FRAME")
        .unwrap()
        .clone();
    let mut nodes = doc.nodes.clone();
    let mut parent = page;
    for i in 0..20_000u32 {
        let id = figstudio_fig::Guid(99, i + 1);
        let mut data = template.data.clone();
        data.set("guid", id.to_val());
        let mut parent_index = figstudio_fig::Obj::new();
        parent_index.insert("guid", parent.to_val());
        parent_index.insert("position", Val::Str(format!("{i:06}")));
        data.set("parentIndex", Val::Obj(parent_index));
        nodes.push(figstudio_fig::Node::from_data(data).unwrap());
        parent = id;
    }
    doc.set_nodes(nodes).unwrap();
    doc.save(&deep).unwrap();
    let tree = assert_success(cli(&["tree", deep.to_str().unwrap(), "--depth", "1"]));
    assert!(tree.lines().count() < 100);
}

#[test]
fn invalid_page_node_and_background_are_runtime_errors() {
    for args in [
        vec!["tree", FIXTURE, "--page", "999"],
        vec!["render", FIXTURE, "--node", "999:999"],
        vec!["render", FIXTURE, "--background", "purple"],
    ] {
        let output = cli(&args);
        assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
        assert!(!text(&output.stderr).contains("USAGE:"));
    }
}

#[test]
fn render_and_thumbnail_write_readable_pngs() {
    let dir = TempDir::new();
    let render = dir.file("render.png");
    let output = cli(&["render", FIXTURE, "-o", render.to_str().unwrap()]);
    let stdout = assert_success(output);
    let size = stdout
        .lines()
        .next()
        .and_then(|line| line.rsplit_once(' '))
        .map(|(_, size)| size)
        .expect("render prints output dimensions");
    let (width, height) = size.split_once('x').expect("width x height");
    assert_png(
        &render,
        Some((width.parse().unwrap(), height.parse().unwrap())),
    );

    let thumb = dir.file("thumbnail.png");
    assert_success(cli(&["thumbnail", FIXTURE, "-o", thumb.to_str().unwrap()]));
    assert_png(&thumb, None);
}

fn assert_png(path: &PathBuf, expected_dimensions: Option<(u32, u32)>) {
    let image = image::open(path).unwrap_or_else(|error| panic!("invalid PNG: {error}"));
    let dimensions = image.into_rgba8().dimensions();
    assert!(dimensions.0 > 0 && dimensions.1 > 0, "empty PNG dimensions");
    if let Some(expected) = expected_dimensions {
        assert_eq!(dimensions, expected, "render output dimensions");
    }
}

#[test]
fn resave_and_synth_outputs_reopen_with_expected_content() {
    let dir = TempDir::new();
    let resaved = dir.file("resaved.fig");
    let stdout = assert_success(cli(&["resave", FIXTURE, "-o", resaved.to_str().unwrap()]));
    assert!(stdout.contains("reads back identical"), "{stdout}");
    let info = assert_success(cli(&["info", resaved.to_str().unwrap()]));
    assert!(info.contains("nodes:    26"), "{info}");
    assert!(info.contains("1. Page 1"), "{info}");

    let synthesized = dir.file("synth.fig");
    assert_success(cli(&[
        "synth",
        FIXTURE,
        "-o",
        synthesized.to_str().unwrap(),
        "--copies",
        "2",
        "--pages",
        "2",
    ]));
    let output = Document::open(&synthesized).unwrap();
    let copies: Vec<_> = output
        .nodes
        .iter()
        .filter(|node| node.id.0 == 3_000_000_000)
        .collect();
    let local_ids: std::collections::HashSet<_> = copies.iter().map(|node| node.id.1).collect();
    assert_eq!(copies.len(), 23);
    assert_eq!(local_ids.len(), copies.len(), "copied local ids collided");
    let info = assert_success(cli(&["info", synthesized.to_str().unwrap()]));
    assert!(info.contains("nodes:    96"), "{info}");
    assert!(info.contains("1. Page 1"), "{info}");
    assert!(info.contains("2. Stress page 2"), "{info}");
}

#[test]
fn trace_logging_goes_to_stderr_only_and_leaks_no_document_names() {
    let plain = cli(&["info", FIXTURE]);
    let traced = Command::new(env!("CARGO_BIN_EXE_figstudio-cli"))
        .args(["info", FIXTURE])
        .env("RUST_LOG", "trace")
        .output()
        .unwrap();
    assert_eq!(traced.status.code(), Some(0), "{}", text(&traced.stderr));
    assert_eq!(plain.stdout, traced.stdout, "stdout must be data only");

    let stderr = text(&traced.stderr);
    assert!(!stderr.is_empty(), "RUST_LOG=trace should log something");
    let doc = Document::open(FIXTURE).unwrap();
    let names: Vec<&str> = doc
        .nodes
        .iter()
        .map(|n| n.name.as_str())
        .filter(|n| n.len() >= 4)
        .collect();
    assert!(!names.is_empty());
    for name in names {
        assert!(!stderr.contains(name), "stderr leaks {name:?}:\n{stderr}");
    }
}

#[test]
fn truncated_fig_fails_with_a_one_line_message() {
    let dir = TempDir::new();
    let mut bytes = fs::read(FIXTURE).unwrap();
    bytes.truncate(bytes.len() / 2);
    let truncated = dir.file("truncated.fig");
    fs::write(&truncated, bytes).unwrap();

    let output = cli(&["info", truncated.to_str().unwrap()]);
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(output.stdout.is_empty());
    assert_eq!(stderr.trim_end().lines().count(), 1, "{stderr}");
    assert!(stderr.starts_with("figstudio-cli: "), "{stderr}");
}

#[test]
fn missing_file_names_its_cause_once() {
    let dir = TempDir::new();
    let missing = dir.file("absent.fig");

    let output = cli(&["info", missing.to_str().unwrap()]);
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    // The decoder's message, then the OS cause, and the cause not repeated.
    assert!(
        stderr.starts_with("figstudio-cli: cannot read file: "),
        "{stderr}"
    );
    assert_eq!(stderr.matches("(os error").count(), 1, "{stderr}");
}
