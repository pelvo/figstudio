//! Missing vector fills must be drawn or reported, not confused with stroke-only vectors.
mod common;
use common::*;
use figstudio_fig::Val;
use figstudio_render::{Renderer, Viewport};
use vello_cpu::kurbo::Affine;

#[test]
fn only_visible_vector_fills_without_paths_are_counted() {
    for geometry in [None, Some(Val::Arr(vec![]))] {
        for (fills, skipped) in [
            (None, 0),
            (Some(Val::Arr(vec![])), 0),
            (Some(paint(1.0, 0.0, 0.0)), 1),
            (Some(hidden_paint("visible", Val::Bool(false))), 0),
            (Some(hidden_paint("opacity", Val::Float(0.0))), 0),
        ] {
            let mut vector = node(2, 1, "VECTOR");
            if let Some(fills) = fills {
                vector.data.set("fillPaints", fills);
            }
            if let Some(geometry) = &geometry {
                vector.data.set("fillGeometry", geometry.clone());
            }
            vector.data.set("strokePaints", paint(0.0, 0.0, 0.0));
            let doc = document(vec![node(1, 0, "CANVAS"), vector]);
            let rendered = Renderer::new().render(&doc, id(1), &viewport());
            assert_eq!(rendered.stats.unsupported_paints, skipped);
        }
    }
}

fn hidden_paint(key: &str, value: Val) -> Val {
    let mut paints = paint(1.0, 0.0, 0.0).items().to_vec();
    paints[0].set(key, value);
    Val::Arr(paints)
}

fn square_blob(style: u32, nonzero: bool) -> Vec<u8> {
    let mut blob = Vec::new();
    let u = |blob: &mut Vec<u8>, n: u32| blob.extend(n.to_le_bytes());
    for n in [4, 4, 1] {
        u(&mut blob, n);
    }
    for (x, y) in [(0.0_f32, 0.0_f32), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
        u(&mut blob, 0);
        blob.extend(x.to_le_bytes());
        blob.extend(y.to_le_bytes());
    }
    for (start, end) in [(0, 1), (2, 1), (2, 3), (0, 3)] {
        for n in [0, start, 0, 0, end, 0, 0] {
            u(&mut blob, n);
        }
    }
    for n in [(style << 1) | u32::from(nonzero), 1, 4, 0, 1, 2, 3] {
        u(&mut blob, n);
    }
    blob
}

fn network_doc(blob: Vec<u8>, style: Option<Val>) -> figstudio_fig::Document {
    let mut vector = node(2, 1, "VECTOR");
    vector.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    vector.data.set(
        "vectorData",
        obj(&[
            ("vectorNetworkBlob", Val::Int(0)),
            ("normalizedSize", size(10.0, 10.0)),
            ("styleOverrideTable", Val::Arr(style.into_iter().collect())),
        ]),
    );
    let mut doc = document(vec![node(1, 0, "CANVAS"), vector]);
    doc.blobs = vec![blob];
    doc
}

#[test]
fn network_regions_fill_scaled_pixels_and_use_style_paints() {
    for (style, color) in [
        (None, [255, 0, 0, 255]),
        (
            Some(obj(&[
                ("styleID", Val::Int(2)),
                ("fillPaints", paint(0.0, 1.0, 0.0)),
            ])),
            [0, 255, 0, 255],
        ),
    ] {
        let doc = network_doc(
            square_blob(if style.is_some() { 2 } else { 0 }, true),
            style,
        );
        let rendered = Renderer::new().render(&doc, id(1), &viewport());
        let px = rendered.pixmap.data_as_u8_slice();
        assert_eq!(pixel(px, 30, 30), color, "network scales to node size");
        assert_eq!(pixel(px, 50, 30), [0, 0, 0, 0]);
        assert_eq!(rendered.stats.unsupported_paints, 0);
    }
}

#[test]
fn malformed_networks_are_not_drawn_and_are_counted() {
    let blob = square_blob(0, true);
    let mut broken = Vec::new();
    for len in 0..blob.len() {
        broken.push(blob[..len].to_vec());
    }
    let mut bad_count = blob.clone();
    bad_count[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    broken.push(bad_count);
    let mut bad_index = blob.clone();
    let last = bad_index.len() - 4;
    bad_index[last..].copy_from_slice(&u32::MAX.to_le_bytes());
    broken.push(bad_index);
    for (at, word) in [
        (64, u32::MAX),
        (64, 2),
        (64, 1),
        (4, u32::MAX),
        (8, u32::MAX),
        (16, f32::NAN.to_bits()),
    ] {
        let mut invalid = blob.clone();
        invalid[at..at + 4].copy_from_slice(&word.to_le_bytes());
        broken.push(invalid);
    }
    for blob in broken {
        let doc = network_doc(blob, None);
        let rendered = Renderer::new().render(&doc, id(1), &viewport());
        assert_eq!(
            pixel(rendered.pixmap.data_as_u8_slice(), 20, 20),
            [0, 0, 0, 0]
        );
        assert!(rendered.stats.unsupported_paints > 0);
    }
}

#[test]
fn network_regions_honour_winding_rules() {
    for (nonzero, alpha) in [(false, 0), (true, 255)] {
        let mut blob = square_blob(0, nonzero);
        let loops_at = blob.len() - 24;
        blob[loops_at..loops_at + 4].copy_from_slice(&2_u32.to_le_bytes());
        // Two equal, same-direction loops cancel only with the odd winding rule.
        for n in [4_u32, 0, 1, 2, 3] {
            blob.extend(n.to_le_bytes());
        }
        let doc = network_doc(blob, None);
        let rendered = Renderer::new().render(&doc, id(1), &viewport());
        assert_eq!(pixel(rendered.pixmap.data_as_u8_slice(), 20, 20)[3], alpha);
        assert_eq!(rendered.stats.unsupported_paints, 0);
    }
}

#[test]
fn network_cubic_handles_are_reversed_with_the_segment() {
    for reverse in [false, true] {
        let mut blob = square_blob(0, true);
        let segment = 60;
        let tangent = if reverse { segment + 24 } else { segment + 12 };
        blob[tangent..tangent + 4].copy_from_slice(&10.0_f32.to_le_bytes());
        let tangent = if reverse { segment + 12 } else { segment + 24 };
        blob[tangent..tangent + 4].copy_from_slice(&10.0_f32.to_le_bytes());
        if reverse {
            blob[segment + 4..segment + 8].copy_from_slice(&1_u32.to_le_bytes());
            blob[segment + 16..segment + 20].copy_from_slice(&0_u32.to_le_bytes());
        }
        let doc = network_doc(blob, None);
        let rendered = Renderer::new().render(&doc, id(1), &viewport());
        let px = rendered.pixmap.data_as_u8_slice();
        assert_eq!(pixel(px, 20, 10)[3], 0, "curved edge cuts out upper centre");
        assert_eq!(pixel(px, 20, 35), [255, 0, 0, 255]);
        assert_eq!(rendered.stats.unsupported_paints, 0);
    }
}

#[test]
fn unsupported_network_styles_remain_counted() {
    for mut blob in [square_blob(2, true), square_blob(0, true)] {
        if blob[blob.len() - 28] == 1 {
            blob[12..16].copy_from_slice(&2_u32.to_le_bytes());
        }
        let doc = network_doc(blob, None);
        let rendered = Renderer::new().render(&doc, id(1), &viewport());
        assert_eq!(
            pixel(rendered.pixmap.data_as_u8_slice(), 20, 20),
            [255, 0, 0, 255]
        );
        assert!(rendered.stats.unsupported_paints > 0);
    }
}

#[test]
fn region_style_can_supply_the_only_visible_fill() {
    let style = obj(&[
        ("styleID", Val::Int(2)),
        ("fillPaints", paint(0.0, 1.0, 0.0)),
    ]);
    let mut doc = network_doc(square_blob(2, true), Some(style));
    let mut nodes = doc.nodes.clone();
    nodes
        .iter_mut()
        .find(|n| n.id == id(2))
        .unwrap()
        .data
        .set("fillPaints", Val::Arr(vec![]));
    doc.set_nodes(nodes).unwrap();
    let rendered = Renderer::new().render(&doc, id(1), &viewport());
    assert_eq!(
        pixel(rendered.pixmap.data_as_u8_slice(), 20, 20),
        [0, 255, 0, 255]
    );
    assert_eq!(rendered.stats.unsupported_paints, 0);
}

fn viewport() -> Viewport {
    Viewport {
        width: 80,
        height: 80,
        transform: Affine::IDENTITY,
        background: None,
    }
}
