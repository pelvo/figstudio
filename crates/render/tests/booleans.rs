//! Boolean operands remain editable but only their computed result is painted.
mod common;
use common::*;
use figstudio_fig::{Document, Val};
use figstudio_render::{Renderer, Viewport, kurbo::Affine};

fn boolean_doc(operation: &str, geometry: Option<&str>) -> Document {
    let mut result = node(2, 1, "BOOLEAN_OPERATION");
    result
        .data
        .set("booleanOperation", Val::Str(operation.into()));
    result.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    result.data.set("strokePaints", paint(1.0, 0.0, 0.0));
    let mut first = node(3, 2, "RECTANGLE");
    first.data.set("fillPaints", paint(0.5, 0.5, 0.5));
    let mut second = node(4, 2, "BOOLEAN_OPERATION");
    second
        .data
        .set("booleanOperation", Val::Str("UNION".into()));
    second.data.set("fillPaints", paint(0.5, 0.5, 0.5));
    let mut operand = node(5, 4, "RECTANGLE");
    operand.data.set("fillPaints", paint(0.5, 0.5, 0.5));
    let mut doc = document(vec![node(1, 0, "CANVAS"), result, first, second, operand]);
    // Stored geometry uses node-local coordinates.
    let mut commands = Vec::new();
    for (op, x, y) in [
        (1, 0.0_f32, 0.0_f32),
        (2, 40.0, 0.0),
        (2, 40.0, 40.0),
        (2, 0.0, 40.0),
    ] {
        commands.push(op);
        commands.extend(x.to_le_bytes());
        commands.extend(y.to_le_bytes());
    }
    commands.push(0);
    let index = doc.blobs.len() as u32;
    doc.blobs.push(commands);
    if let Some(field) = geometry {
        let paths = Val::Arr(vec![obj(&[("commandsBlob", Val::Int(i64::from(index)))])]);
        doc.nodes[1].data.set(field, paths.clone());
        doc.nodes[3].data.set(field, paths);
    }
    doc
}

fn render(doc: &Document) -> figstudio_render::Frame {
    Renderer::new().render(
        doc,
        id(1),
        &Viewport {
            width: 80,
            height: 80,
            transform: Affine::IDENTITY,
            background: None,
        },
    )
}

#[test]
fn computed_boolean_uses_its_own_paints_without_operand_pixels() {
    for operation in ["UNION", "SUBTRACT", "INTERSECT", "EXCLUDE"] {
        for field in ["fillGeometry", "strokeGeometry"] {
            let doc = boolean_doc(operation, Some(field));
            let frame = render(&doc);
            let px = frame.pixmap.data_as_u8_slice();
            assert_eq!(pixel(px, 20, 20), [255, 0, 0, 255], "{operation} {field}");
            assert!(
                !px.as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[3] > 0 && p[0] == p[1]),
                "operand pixels"
            );
            assert!(
                !frame.stats.to_string().contains("approximated"),
                "{}",
                frame.stats
            );
            #[cfg(feature = "parallel")]
            {
                let vp = Viewport {
                    width: 80,
                    height: 80,
                    transform: Affine::IDENTITY,
                    background: None,
                };
                let (recording, _) = Renderer::new().record(
                    &doc,
                    id(1),
                    Affine::IDENTITY,
                    figstudio_render::kurbo::Rect::new(0.0, 0.0, 80.0, 80.0),
                    usize::MAX,
                );
                let (tile, _) = recording.unwrap().render(&vp).unwrap();
                assert_eq!(
                    pixel(tile.pixmap.data_as_u8_slice(), 20, 20),
                    [255, 0, 0, 255]
                );
            }
            // Selection can still drill into the operands even though they are not painted.
            assert_eq!(
                figstudio_render::hit_test(&doc, id(1), (20.0, 20.0).into()),
                Some(id(5))
            );
        }
    }
}

#[test]
fn boolean_without_computed_geometry_draws_operands_and_reports_approximation() {
    let mut doc = boolean_doc("UNION", None);
    // Only the outer boolean is missing its result; the inner operand is an ordinary shape.
    let inner = &mut doc.nodes[3];
    inner.kind = "RECTANGLE".into();
    inner.data.set("type", Val::Str("RECTANGLE".into()));
    let frame = render(&doc);
    assert_eq!(
        pixel(frame.pixmap.data_as_u8_slice(), 20, 20),
        [128, 128, 128, 255]
    );
    assert!(
        frame
            .stats
            .to_string()
            .contains("1 boolean operations approximated"),
        "{}",
        frame.stats
    );
    let mut summary = figstudio_render::Stats::default();
    summary.max_with(&frame.stats);
    assert!(
        summary
            .to_string()
            .contains("1 boolean operations approximated")
    );
}

#[test]
fn computed_boolean_masks_use_result_geometry_not_operand_bounds() {
    let mut doc = boolean_doc("UNION", Some("fillGeometry"));
    doc.nodes[1].data.set("mask", Val::Bool(true));
    doc.nodes[2].data.set("size", size(70.0, 70.0));
    let mut content = node(6, 1, "RECTANGLE");
    content.data.set("size", size(70.0, 70.0));
    content.data.set("fillPaints", paint(0.0, 1.0, 0.0));
    let mut nodes = doc.nodes.clone();
    nodes.push(content);
    doc.set_nodes(nodes).unwrap();
    let frame = render(&doc);
    let px = frame.pixmap.data_as_u8_slice();
    assert_eq!(pixel(px, 20, 20), [0, 255, 0, 255]);
    assert_eq!(pixel(px, 50, 20), [0, 0, 0, 0]);
    assert_eq!(frame.stats.masks, 1);
}
