//! A layer's animation exported as files: one pass of its timeline, every frame drawn at its
//! time, the size following the scale.

use std::io::{Cursor, Read};

use figstudio_fig::Document;
use figstudio_mcp_core::motion_export::{export, plan};
use figstudio_mcp_core::rendering::Rendering;
use figstudio_video::Format;

fn motion() -> Document {
    Document::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/motion.fig"
    ))
    .unwrap()
}

/// "Object" (1:1106) plays in the 1.5 s looping timeline of 1:1105.
const ANIMATED: &str = "1:1106";

#[test]
fn a_pass_is_planned_from_the_timeline() {
    let doc = motion();
    let p = plan(&doc, ANIMATED, Format::Frames, Some(10), Some(0.5)).unwrap();
    assert_eq!(p.root.to_string(), "1:1105", "the timeline's root is drawn");
    assert_eq!(p.frames, (p.period_us / 1e6 * 10.0).round() as u32);
    assert!(p.frames > 1);
    let b = figstudio_render::node_bounds(&doc, p.root).unwrap();
    assert_eq!(u32::from(p.width), (b.width() * 0.5).round() as u32);
    // A still layer has nothing to export.
    let still = doc
        .nodes
        .iter()
        .find(|n| n.kind == "TEXT")
        .unwrap()
        .id
        .to_string();
    assert!(plan(&doc, &still, Format::Gif, None, None).is_err());
}

#[test]
fn png_frames_change_over_the_timeline() {
    let doc = motion();
    let done = export(
        &doc,
        ANIMATED,
        Format::Frames,
        Some(10),
        Some(0.5),
        &Rendering::default(),
    )
    .unwrap();
    let (bytes, p) = (done.bytes, done.plan);
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert_eq!(zip.len(), p.frames as usize);
    let mut frame = |i: u32| {
        let mut png = Vec::new();
        zip.by_name(&format!("frame_{:05}.png", i + 1))
            .unwrap()
            .read_to_end(&mut png)
            .unwrap();
        image::load_from_memory(&png).unwrap().to_rgba8()
    };
    let (first, middle) = (frame(0), frame(p.frames / 2));
    assert_eq!(
        first.dimensions(),
        (u32::from(p.width), u32::from(p.height))
    );
    assert_ne!(
        first.as_raw(),
        middle.as_raw(),
        "the animation moves between frames"
    );
}

#[test]
fn every_format_comes_out() {
    let doc = motion();
    // Video needs the `video` feature (the browser build has GIF and PNG frames only).
    for format in [Format::Gif, Format::Webm, Format::Mp4]
        .into_iter()
        .filter(|f| f.available())
    {
        let done = export(
            &doc,
            ANIMATED,
            format,
            Some(10),
            Some(0.25),
            &Rendering::default(),
        )
        .unwrap_or_else(|e| panic!("{format:?}: {e}"));
        let (bytes, p) = (done.bytes, done.plan);
        assert!(bytes.len() > 100, "{format:?}");
        if format != Format::Gif {
            assert_eq!(p.width % 2, 0, "video sides are even");
        }
    }
}
