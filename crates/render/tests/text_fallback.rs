//! Text the requested font cannot draw falls back to a face that can: symbols (⌞) and
//! emoji must not be dropped just because a color-emoji font maps them.
mod common;
use common::*;
use figstudio_fig::{Document, Val};
use figstudio_render::Renderer;

/// A TEXT node laid out by system fonts (no stored glyphs), in black on white.
fn doc_with(chars: &str) -> Document {
    let mut text = node(3, 1, "TEXT");
    text.data.set("fillPaints", paint(1.0, 1.0, 1.0));
    text.data.set(
        "textData",
        obj(&[
            ("characters", Val::Str(chars.into())),
            (
                "fontName",
                obj(&[
                    ("family", Val::Str("Inter".into())),
                    ("style", Val::Str("Regular".into())),
                ]),
            ),
            ("textAlignHorizontal", Val::Str("LEFT".into())),
            ("textAlignVertical", Val::Str("TOP".into())),
        ]),
    );
    text.data.set("fontSize", Val::Float(40.0));
    document(vec![node(1, 0, "CANVAS"), text])
}

/// Renders the node's own pixels with system fonts, at 1:1 in a 60x60 viewport.
fn laid_out(chars: &str) -> (usize, usize) {
    let doc = doc_with(chars);
    let mut r = Renderer::new();
    r.system_fonts = true;
    let frame = r.render(
        &doc,
        id(1),
        &figstudio_render::Viewport {
            width: 60,
            height: 60,
            transform: Default::default(),
            background: None,
        },
    );
    let ink = frame
        .pixmap
        .data()
        .iter()
        .filter(|p| p.r > 128 && p.g > 128 && p.b > 128)
        .count();
    (frame.stats.text_system_fonts, ink)
}

#[test]
fn symbols_and_emoji_are_drawn_by_a_fallback_face() {
    for chars in ["⌞", "⚠", "★", "😀"] {
        let (draws, ink) = laid_out(chars);
        assert_eq!(draws, 1, "{chars:?} laid out");
        assert!(ink > 0, "{chars:?} drew no pixels");
    }
}
