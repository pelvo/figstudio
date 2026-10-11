mod common;
use common::*;
use figstudio_fig::{Document, Val};

fn part(local: i64, fields: &[(&str, Val)]) -> Val {
    let mut v = obj(fields);
    let guid = obj(&[
        ("sessionID", Val::Int(40_000_000)),
        ("localID", Val::Int(local)),
    ]);
    v.set("guidPath", obj(&[("guids", Val::Arr(vec![guid]))]));
    v
}

/// A connector keeps its line only as generated parts: geometry in
/// `derivedImmutableFrameData`, the stroke paint in `nodeGenerationData`.
#[test]
fn connector_line_is_drawn_from_its_generated_parts() {
    let mut doc: Document = document(vec![]);
    let blob = doc.blobs.len() as i64;
    let mut b = Vec::new();
    for (cmd, x, y) in [
        (1u8, 10.0f32, 30.0f32),
        (2, 70.0, 30.0),
        (2, 70.0, 34.0),
        (2, 10.0, 34.0),
    ] {
        b.push(cmd);
        b.extend(x.to_le_bytes());
        b.extend(y.to_le_bytes());
    }
    b.push(0);
    doc.blobs.push(b);
    let geom = Val::Arr(vec![obj(&[("commandsBlob", Val::Int(blob))])]);
    let mut c = node(2, 1, "CONNECTOR");
    c.data.set("size", size(80.0, 80.0));
    // Like Figma: the connector itself has no stroke; its line is a generated part.
    c.data.set(
        "derivedImmutableFrameData",
        obj(&[(
            "overrides",
            Val::Arr(vec![part(
                0,
                &[("size", size(80.0, 80.0)), ("strokeGeometry", geom)],
            )]),
        )]),
    );
    c.data.set(
        "nodeGenerationData",
        obj(&[(
            "overrides",
            Val::Arr(vec![part(0, &[("strokePaints", paint(1.0, 0.0, 0.0))])]),
        )]),
    );
    doc.set_nodes(vec![node(1, 0, "CANVAS"), c]).unwrap();
    let px = pixels(&doc);
    assert_eq!(pixel(&px, 40, 32), [255, 0, 0, 255], "line drawn");
    assert_ne!(
        pixel(&px, 40, 40),
        [255, 0, 0, 255],
        "nothing outside the line"
    );
}
