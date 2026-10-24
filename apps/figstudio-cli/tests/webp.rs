//! Image-format contracts at the CLI boundary.
use std::{fs, process::Command};

fn cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_figstudio-cli"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn formats_reach_cli_images_and_compare_reports() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/structure.fig"
    );
    for command in ["render", "thumbnail"] {
        for (name, explicit, expected) in [
            ("inferred.webp", None, image::ImageFormat::WebP),
            ("explicit.png", Some("webp"), image::ImageFormat::WebP),
            ("override.webp", Some("png"), image::ImageFormat::Png),
            ("default.png", None, image::ImageFormat::Png),
        ] {
            let path = dir.path().join(name);
            let mut args = vec![command, fixture, "-o", path.to_str().unwrap()];
            if let Some(format) = explicit {
                args.extend(["--format", format]);
            }
            let output = cli(&args);
            assert!(output.status.success(), "{:?}", output);
            let bytes = fs::read(path).unwrap();
            assert_eq!(image::guess_format(&bytes).unwrap(), expected);
            assert!(image::load_from_memory(&bytes).unwrap().width() > 0);
        }
    }
    for format in ["png", "webp"] {
        let output = Command::new(env!("CARGO_BIN_EXE_figstudio-cli"))
            .current_dir(dir.path())
            .args(["render", fixture, "--format", format, "--no-system-fonts"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(
            image::load_from_memory(
                &fs::read(dir.path().join(format!("structure.{format}"))).unwrap()
            )
            .unwrap()
            .width()
                > 0
        );
    }
    let mut doc = figstudio_fig::Document::open(fixture).unwrap();
    let mut nodes = doc.nodes.clone();
    let frame = nodes.iter_mut().find(|n| n.kind == "FRAME").unwrap();
    frame
        .data
        .set("name", figstudio_fig::Val::Str("format-test".into()));
    let mut color = figstudio_fig::Obj::new();
    for (k, v) in [("r", 1.0), ("g", 0.0), ("b", 1.0), ("a", 1.0)] {
        color.insert(k, figstudio_fig::Val::Float(v));
    }
    let mut paint = figstudio_fig::Obj::new();
    paint.insert("type", figstudio_fig::Val::Enum("SOLID"));
    paint.insert("color", figstudio_fig::Val::Obj(color));
    frame.data.set(
        "fillPaints",
        figstudio_fig::Val::Arr(vec![figstudio_fig::Val::Obj(paint)]),
    );
    *frame = figstudio_fig::Node::from_data(frame.data.clone()).unwrap();
    doc.set_nodes(nodes).unwrap();
    let changed = dir.path().join("changed.fig");
    doc.save(&changed).unwrap();
    for format in ["webp", "png"] {
        let out = dir.path().join(format);
        let mut args = vec![
            "compare",
            fixture,
            changed.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
            "--no-system-fonts",
        ];
        if format == "png" {
            args.extend(["--format", format]);
        }
        let output = cli(&args);
        assert!(output.status.success(), "{:?}", output);
        let json = fs::read_to_string(out.join("report.json")).unwrap();
        let html = fs::read_to_string(out.join("report.html")).unwrap();
        assert!(json.contains(&format!(".{format}\"")));
        assert!(html.contains(&format!(".{format}\"")));
        let images: Vec<_> = fs::read_dir(out.join("render"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert!(!images.is_empty());
        for path in images {
            assert_eq!(path.extension().unwrap(), format);
            let bytes = fs::read(path).unwrap();
            assert_eq!(
                image::guess_format(&bytes).unwrap(),
                if format == "webp" {
                    image::ImageFormat::WebP
                } else {
                    image::ImageFormat::Png
                }
            );
        }
    }
}

#[test]
fn invalid_format_is_usage_error_before_opening_file() {
    for command in ["render", "thumbnail", "compare"] {
        for format in ["jxl", "jpg", "", "WEBP"] {
            let output = cli(&[command, "missing.fig", "-o", "out", "--format", format]);
            assert_eq!(output.status.code(), Some(2));
            assert!(String::from_utf8_lossy(&output.stderr).contains("USAGE:"));
        }
    }
}
