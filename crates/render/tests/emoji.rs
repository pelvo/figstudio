//! Stored emoji markers are not drawable outlines.
mod common;
use common::*;
use figstudio_fig::{Document, Val};
use figstudio_render::{Renderer, Viewport};
use vello_cpu::kurbo::Affine;

fn emoji_document(points: &[i64]) -> Document {
    let mut text = node(3, 1, "TEXT");
    text.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let root = figstudio_fig::Node::from_data(obj(&[
        ("guid", id(0).to_val()),
        ("type", Val::Enum("DOCUMENT")),
    ]))
    .unwrap();
    let mut doc = document(vec![root, node(1, 0, "CANVAS"), text.clone()]);
    let blob = doc.blobs.len() as i64;
    let mut commands = Vec::new();
    for (cmd, x, y) in [
        (1, 0f32, -0.07f32),
        (2, 1.0, -0.07),
        (2, 1.0, 0.93),
        (2, 0.0, 0.93),
    ] {
        commands.push(cmd);
        commands.extend(x.to_le_bytes());
        commands.extend(y.to_le_bytes());
    }
    commands.push(0);
    doc.blobs.push(commands);
    text.data.set(
        "derivedTextData",
        obj(&[(
            "glyphs",
            Val::Arr(vec![obj(&[
                ("commandsBlob", Val::Int(blob)),
                ("fontSize", Val::Float(40.0)),
                ("position", size(10.0, 50.0)),
                (
                    "emojiCodePoints",
                    Val::Arr(points.iter().copied().map(Val::Int).collect()),
                ),
            ])]),
        )]),
    );
    let mut nodes = doc.nodes.clone();
    *nodes.iter_mut().find(|n| n.id == id(3)).unwrap() = text;
    doc.set_nodes(nodes).unwrap();
    doc
}

struct Square(std::sync::Mutex<Vec<(String, u16)>>, bool);

impl figstudio_render::EmojiSource for Square {
    fn raster(&self, cluster: &str, pixels: u16) -> Option<figstudio_render::EmojiRaster> {
        self.0.lock().unwrap().push((cluster.to_owned(), pixels));
        let side = pixels / 2;
        figstudio_render::EmojiRaster::from_rgba(
            [0, 200, 40, 255].repeat(usize::from(side).pow(2)),
            side,
            side,
            f64::from(pixels) / 10.0 + 0.25,
            -f64::from(pixels) * 0.75 + 0.5,
        )
        .map(|raster| {
            if self.1 {
                raster.with_verified_shaping()
            } else {
                raster
            }
        })
    }
}

#[test]
fn source_attestation_controls_counts_not_cluster_pixels_or_cache() {
    for (points, cluster) in [
        (&[0x2764][..], "❤"),
        (&[0x2764, 0xFE0F, 0x200D, 0x2764][..], "❤\u{fe0f}\u{200d}❤"),
    ] {
        for verified in [false, true] {
            let doc = emoji_document(points);
            let source = std::sync::Arc::new(Square(Default::default(), verified));
            let mut renderer = Renderer::new();
            renderer.system_fonts = false;
            renderer.set_emoji_source(Some(source.clone()));
            let frame = renderer.render(&doc, id(1), &viewport());
            let at = |x: usize, y: usize| {
                &frame.pixmap.data_as_u8_slice()[(y * 80 + x) * 4..(y * 80 + x) * 4 + 4]
            };
            assert_eq!(at(20, 25), &[0, 200, 40, 255]);
            assert_eq!(at(10, 25)[3], 0);
            assert_eq!(at(20, 45)[3], 0);
            assert!((1..255).contains(&at(14, 21)[3]), "fractional left bearing");
            assert!(
                (1..255).contains(&at(20, 20)[3]),
                "fractional baseline offset"
            );
            assert_eq!(at(13, 21)[3], 0);
            assert_eq!(at(20, 19)[3], 0);
            assert_eq!(frame.stats.emoji_skipped, 0);
            assert_eq!(frame.stats.emoji_approximated, usize::from(!verified));
            let again = renderer.render(&doc, id(1), &viewport());
            assert_eq!(again.stats, frame.stats);
            assert_eq!(
                renderer.emoji_counts(&doc).emoji_approximated,
                usize::from(!verified)
            );
            assert_eq!(
                frame.pixmap.data_as_u8_slice(),
                again.pixmap.data_as_u8_slice()
            );
            assert_eq!(*source.0.lock().unwrap(), vec![(cluster.into(), 40)]);
            let mut zoomed = viewport();
            zoomed.transform = Affine::scale(2.0);
            let zoomed = renderer.render(&doc, id(1), &zoomed);
            assert_eq!(
                &zoomed.pixmap.data_as_u8_slice()[(50 * 80 + 40) * 4..][..4],
                &[0, 200, 40, 255]
            );
            assert_eq!(source.0.lock().unwrap().len(), 2);
            #[cfg(feature = "parallel")]
            {
                let (recording, _) = renderer.record(
                    &doc,
                    id(1),
                    Affine::IDENTITY,
                    vello_cpu::kurbo::Rect::new(0.0, 0.0, 80.0, 80.0),
                    1_000_000,
                );
                let recording = recording.unwrap();
                let replay = recording.render(&viewport()).unwrap().0;
                assert_eq!(
                    frame.pixmap.data_as_u8_slice(),
                    replay.pixmap.data_as_u8_slice()
                );
                assert_eq!(frame.stats, replay.stats);
                #[cfg(feature = "gpu")]
                assert!(matches!(
                    recording.gpu_scene(&viewport(), 4096),
                    Some(Err(figstudio_render::GpuUnsupported::Image))
                ));
            }
            renderer.set_emoji_source(None);
            assert_eq!(
                renderer
                    .render(&doc, id(1), &viewport())
                    .stats
                    .emoji_skipped,
                1
            );
        }
    }
}

#[test]
fn empty_or_invalid_platform_rasters_are_rejected() {
    use figstudio_render::EmojiRaster;
    assert!(EmojiRaster::from_rgba(vec![0; 16], 2, 2, 0.0, -2.0).is_none());
    assert!(EmojiRaster::from_rgba(vec![255; 16], 2, 2, f64::NAN, -2.0).is_none());
    assert!(EmojiRaster::from_rgba(vec![255; 15], 2, 2, 0.0, -2.0).is_none());
}

fn viewport() -> Viewport {
    Viewport {
        width: 80,
        height: 80,
        transform: Affine::IDENTITY,
        background: None,
    }
}

#[test]
fn emoji_without_fonts_never_fills_the_placeholder() {
    let doc = emoji_document(&[0x1F4F1]);
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    let frame = renderer.render(&doc, id(1), &viewport());
    assert!(
        frame
            .pixmap
            .data_as_u8_slice()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[3] == 0),
        "emoji marker painted a square"
    );
    assert_eq!(frame.stats.emoji_skipped, 1);
    assert_eq!(frame.stats.emoji_approximated, 0);
    assert_eq!(
        frame.stats.text_missing, 0,
        "do not lay out an emoji-only run again"
    );
    assert!(frame.stats.to_string().contains("1 emoji skipped"));
}

#[test]
fn an_empty_marker_keeps_ordinary_outlines_and_truncation_hides_emoji() {
    assert_eq!(
        pixel(&pixels(&emoji_document(&[])), 30, 30),
        &[255, 0, 0, 255]
    );
    let mut doc = emoji_document(&[0x1F4F1]);
    let mut nodes = doc.nodes.clone();
    let text = nodes.iter_mut().find(|n| n.id == id(3)).unwrap();
    text.data
        .get_mut("derivedTextData")
        .unwrap()
        .set("truncationStartIndex", Val::Int(0));
    doc.set_nodes(nodes).unwrap();
    assert!(pixels(&doc).as_chunks::<4>().0.iter().all(|p| p[3] == 0));
}

#[test]
fn mixed_stored_runs_keep_ordinary_glyphs_without_drawing_emoji_boxes() {
    let mut doc = emoji_document(&[0x1F4F1]);
    let mut nodes = doc.nodes.clone();
    let text = nodes.iter_mut().find(|n| n.id == id(3)).unwrap();
    let mut ordinary = text
        .data
        .path(&["derivedTextData", "glyphs"])
        .unwrap()
        .items()[0]
        .clone();
    ordinary.set("emojiCodePoints", Val::Arr(Vec::new()));
    ordinary.set("position", size(50.0, 50.0));
    let Some(Val::Arr(glyphs)) = text
        .data
        .get_mut("derivedTextData")
        .unwrap()
        .get_mut("glyphs")
    else {
        panic!("test glyphs")
    };
    glyphs.push(ordinary);
    doc.set_nodes(nodes).unwrap();
    let px = pixels(&doc);
    assert_eq!(pixel(&px, 30, 30), &[0, 0, 0, 0]);
    assert_eq!(pixel(&px, 60, 30), &[255, 0, 0, 255]);
}

#[test]
#[ignore = "requires an installed colour emoji face (for example Noto Color Emoji)"]
fn installed_colour_emoji_is_not_the_text_paint() {
    let doc = emoji_document(&[0x1F4F1]);
    let mut renderer = Renderer::new();
    let output = renderer.render(&doc, id(1), &viewport());
    let colours: std::collections::HashSet<_> = output
        .pixmap
        .data_as_u8_slice()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] == 255)
        .map(|p| p.to_vec())
        .collect();
    assert!(
        colours.len() > 8,
        "expected a multicolour emoji, not a text-coloured square"
    );
    assert!(!colours.contains(&vec![255, 0, 0, 255]));
    assert_eq!(output.stats.emoji_skipped, 0);
    assert_eq!(output.stats.emoji_approximated, 0);
    #[cfg(feature = "parallel")]
    {
        let recording = renderer
            .record(
                &doc,
                id(1),
                Affine::IDENTITY,
                figstudio_render::kurbo::Rect::new(0.0, 0.0, 80.0, 80.0),
                1 << 20,
            )
            .0
            .unwrap();
        let replayed = recording.render(&viewport()).unwrap().0;
        assert_eq!(
            output.pixmap.data_as_u8_slice(),
            replayed.pixmap.data_as_u8_slice()
        );
        assert_eq!(output.stats, replayed.stats);
        #[cfg(feature = "gpu")]
        assert_eq!(
            recording.gpu_scene(&viewport(), 4096).unwrap().err(),
            Some(figstudio_render::GpuUnsupported::Image)
        );
    }
    let sequence = renderer.render(&emoji_document(&[0x1F4F1, 0xFE0F]), id(1), &viewport());
    assert_eq!(sequence.stats.emoji_approximated, 0);
    assert_eq!(sequence.stats.emoji_skipped, 0);
}

/// A platform emoji font that is still loading: the first raster is empty, later ones draw.
struct Loading(std::sync::atomic::AtomicUsize);

impl figstudio_render::EmojiSource for Loading {
    fn raster(&self, _: &str, pixels: u16) -> Option<figstudio_render::EmojiRaster> {
        if self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
            return None;
        }
        let side = pixels / 2;
        figstudio_render::EmojiRaster::from_rgba(
            [0, 200, 40, 255].repeat(usize::from(side).pow(2)),
            side,
            side,
            0.0,
            -f64::from(side),
        )
    }
}

#[test]
fn a_complete_render_retries_an_emoji_whose_font_was_still_loading() {
    // A complete frame is what a Recents thumbnail stores until the file is opened again.
    let doc = emoji_document(&[0x2764]);
    let source = std::sync::Arc::new(Loading(Default::default()));
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    renderer.set_emoji_source(Some(source.clone()));
    let frame = renderer.render_complete(&doc, id(1), &viewport(), |_| None);
    assert_eq!(
        frame.stats.emoji_skipped, 0,
        "the one retry happened in this call"
    );
    assert_eq!(source.0.load(std::sync::atomic::Ordering::Relaxed), 2);
    // Nothing missed for the first time, so a later complete render draws once.
    renderer.render_complete(&doc, id(1), &viewport(), |_| None);
    assert_eq!(
        source.0.load(std::sync::atomic::Ordering::Relaxed),
        2,
        "cached"
    );
}
