//! Truncated text draws only the glyphs before `truncationStartIndex` (the ellipsis included).
mod common;
use common::*;
use figstudio_fig::Val;

#[test]
fn glyphs_after_the_truncation_index_are_not_drawn() {
    let mut text = node(3, 1, "TEXT");
    text.data.set("fillPaints", paint(0.0, 0.0, 1.0));
    let mut doc = document(vec![node(1, 0, "CANVAS"), text.clone()]);
    // A unit square glyph outline, scaled to 20 px by the glyph font size.
    let blob = doc.blobs.len() as i64;
    let mut commands = Vec::new();
    for (cmd, x, y) in [(1, 0f32, 0f32), (2, 1.0, 0.0), (2, 1.0, 1.0), (2, 0.0, 1.0)] {
        commands.push(cmd);
        commands.extend(x.to_le_bytes());
        commands.extend(y.to_le_bytes());
    }
    commands.push(0);
    doc.blobs.push(commands);
    let glyph = |x: f64, y: f64| {
        obj(&[
            ("commandsBlob", Val::Int(blob)),
            ("fontSize", Val::Float(20.0)),
            ("position", size(x, y)),
        ])
    };
    // Line one: glyph at x 0, then the ellipsis at x 20 (index 1); line two at y 60 (index 2).
    let mut stored = obj(&[(
        "glyphs",
        Val::Arr(vec![glyph(0.0, 20.0), glyph(20.0, 20.0), glyph(0.0, 60.0)]),
    )]);
    stored.set("truncationStartIndex", Val::Int(2));
    text.data.set("derivedTextData", stored);
    let mut nodes = doc.nodes.clone();
    *nodes.iter_mut().find(|n| n.id == id(3)).unwrap() = text;
    doc.set_nodes(nodes).unwrap();
    let px = pixels(&doc);
    assert_eq!(pixel(&px, 10, 10), &[0, 0, 255, 255]);
    assert_eq!(
        pixel(&px, 30, 10),
        &[0, 0, 255, 255],
        "ellipsis glyph is drawn"
    );
    assert_eq!(pixel(&px, 10, 50), &[0, 0, 0, 0], "overflow line is hidden");
}
