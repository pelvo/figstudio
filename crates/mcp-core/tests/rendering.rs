//! The rendering tools in core: PNG output, lazy image reads and the asset directory modes.
use figstudio_fig::Document;
use figstudio_mcp_core::{
    assets::{AssetFormat, download},
    design::{Assets, Options, design_context},
    output::{Block, Output},
    rendering::Rendering,
    screenshot::screenshot,
};
use figstudio_render::images::Images;
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
}

fn text(out: &Output) -> String {
    out.content
        .iter()
        .filter_map(Block::as_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn images(out: &Output) -> Vec<&str> {
    out.content
        .iter()
        .filter_map(|b| match b {
            Block::Image { data_b64, .. } => Some(data_b64.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_screenshot_is_a_png_block_then_a_size_line() {
    let doc = Document::open(fixture()).unwrap();
    let out = screenshot(&doc, "1:34", Some(1.0), &Rendering::default());
    assert!(!out.is_error, "{}", text(&out));
    assert_eq!(images(&out).len(), 1);
    assert!(text(&out).contains(" px, scale 1"));
    assert!(screenshot(&doc, "1:34", Some(9.0), &Rendering::default()).is_error);
}

/// Original bytes of a lazily opened file come from the file through `Rendering::new`, and are
/// the bytes an eager open holds; without a path they count as missing.
#[test]
fn lazy_originals_are_read_from_the_registered_file() {
    let eager = Document::open(fixture()).unwrap();
    let lazy = Document::open_lazy(fixture()).unwrap();
    assert!(lazy.images.is_empty() && !lazy.lazy_images.is_empty());
    let ids = vec!["1:34".to_owned()];
    let read = Rendering::new(fixture(), Images::default());
    let want = download(
        &eager,
        &ids,
        AssetFormat::Png,
        None,
        true,
        &Rendering::default(),
    );
    let got = download(&lazy, &ids, AssetFormat::Png, None, true, &read);
    assert_eq!(want, got);
    assert!(!images(&got).is_empty());
    let none = download(
        &lazy,
        &ids,
        AssetFormat::Png,
        None,
        true,
        &Rendering::default(),
    );
    assert!(
        none.is_error && text(&none).contains("not stored"),
        "{}",
        text(&none)
    );
}

#[test]
fn inline_assets_work_in_every_build_and_files_need_the_fs_feature() {
    let doc = Document::open(fixture()).unwrap();
    let inline = Options {
        file_key: "structure".into(),
        assets: Assets::Inline,
        ..Options::default()
    };
    let out = design_context(&doc, "1:34", &inline);
    assert!(!out.is_error, "{}", text(&out));
    let dir = tempfile_dir();
    let files = Options {
        assets: Assets::dir(dir.clone()),
        ..inline
    };
    let out = design_context(&doc, "1:34", &files);
    #[cfg(feature = "fs")]
    {
        assert!(!out.is_error, "{}", text(&out));
        assert!(std::fs::read_dir(&dir).unwrap().count() > 0);
    }
    #[cfg(not(feature = "fs"))]
    {
        assert!(out.is_error);
        assert_eq!(text(&out), "asset files are not available in this build");
        assert!(!dir.exists());
    }
    let _ = std::fs::remove_dir_all(dir);
}

fn tempfile_dir() -> PathBuf {
    std::env::temp_dir().join(format!("figstudio-core-assets-{}", std::process::id()))
}
