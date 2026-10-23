//! Browser render settings carry supplied faces through the actual draw boundary.
#[path = "../../render/tests/common/mod.rs"]
mod common;
use common::*;
use figstudio_fig::Val;
use figstudio_mcp_core::{rendering::Rendering, screenshot::Root};
use figstudio_render::{Viewport, kurbo::Affine};
use std::sync::Arc;

#[test]
fn supplied_fonts_draw_outline_less_text() {
    let mut text = node(2, 1, "TEXT");
    text.data.set("size", size(200.0, 40.0));
    text.data.set("fontSize", Val::Float(28.0));
    text.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    text.data.set(
        "textData",
        obj(&[("characters", Val::Str("Sample".into()))]),
    );
    let doc = document(vec![node(1, 0, "CANVAS"), text]);
    let viewport = Viewport {
        width: 200,
        height: 40,
        transform: Affine::IDENTITY,
        background: None,
    };
    let mut rendering = Rendering::default();
    let absent = rendering.draw(&doc, Root::Node(id(1)), &viewport);
    assert_eq!(absent.stats.text_missing, 1);
    assert!(absent.pixmap.data_as_u8_slice().iter().all(|&b| b == 0));
    rendering
        .fonts
        .push((Arc::new(epaint_default_fonts::HACK_REGULAR.to_vec()), 0));
    let present = rendering.draw(&doc, Root::Node(id(1)), &viewport);
    assert_eq!(present.stats.text_missing, 0);
    assert_eq!(present.stats.text_system_fonts, 1);
    assert!(
        present
            .pixmap
            .data_as_u8_slice()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 50 && p[3] > 50)
            .count()
            > 100
    );
}
