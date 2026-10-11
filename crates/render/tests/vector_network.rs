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
        assert_eq!(rendered.stats.unsupported_paints, 1);
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

/// Top edge as a cubic with unequal handles; `reverse` stores it end-to-start with the handles
/// swapped to match, so both encodings describe the same curve.
fn cubic_blob(reverse: bool, start_handle: f32, end_handle: f32) -> Vec<u8> {
    let mut blob = square_blob(0, true);
    let segment = 60;
    let (first, second) = if reverse {
        (end_handle, start_handle)
    } else {
        (start_handle, end_handle)
    };
    let mut set = |at: usize, word: u32| blob[at..at + 4].copy_from_slice(&word.to_le_bytes());
    set(segment + 12, first.to_bits());
    set(segment + 24, second.to_bits());
    if reverse {
        set(segment + 4, 1);
        set(segment + 16, 0);
    }
    blob
}

#[test]
fn network_cubic_handles_are_reversed_with_the_segment() {
    // Start handle (0, 8) and end handle (0, 2): swapping them mirrors the curve, so the
    // two probes below only pass when a reversed segment swaps its handles.
    for reverse in [false, true] {
        let blob = cubic_blob(reverse, 8.0, 2.0);
        let doc = network_doc(blob, None);
        let rendered = Renderer::new().render(&doc, id(1), &viewport());
        let px = rendered.pixmap.data_as_u8_slice();
        assert_eq!(pixel(px, 6, 8)[3], 0, "deep cut near the left end");
        assert_eq!(
            pixel(px, 34, 12),
            [255, 0, 0, 255],
            "shallow cut at the right"
        );
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
        assert_eq!(rendered.stats.unsupported_paints, 1);
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

fn with_node_fills(doc: &mut figstudio_fig::Document, fills: Val) {
    let mut nodes = doc.nodes.clone();
    nodes
        .iter_mut()
        .find(|n| n.id == id(2))
        .unwrap()
        .data
        .set("fillPaints", fills);
    doc.set_nodes(nodes).unwrap();
}

fn green_style() -> Val {
    obj(&[
        ("styleID", Val::Int(2)),
        ("fillPaints", paint(0.0, 1.0, 0.0)),
    ])
}

#[test]
fn style_only_fill_is_counted_once_when_reconstruction_fails() {
    let blob = square_blob(2, true);
    let mut truncated = blob.clone();
    truncated.pop();
    for (blob, normalized) in [(truncated, (10.0, 10.0)), (blob, (0.0, 10.0))] {
        let mut doc = network_doc(blob, Some(green_style()));
        let mut nodes = doc.nodes.clone();
        let data = &mut nodes.iter_mut().find(|n| n.id == id(2)).unwrap().data;
        data.set("fillPaints", Val::Arr(vec![]));
        data.set(
            "vectorData",
            obj(&[
                ("vectorNetworkBlob", Val::Int(0)),
                ("normalizedSize", size(normalized.0, normalized.1)),
                ("styleOverrideTable", Val::Arr(vec![green_style()])),
            ]),
        );
        doc.set_nodes(nodes).unwrap();
        let rendered = Renderer::new().render(&doc, id(1), &viewport());
        assert_eq!(rendered.stats.unsupported_paints, 1);
    }
}

#[test]
fn vectors_without_a_visible_fill_do_not_reconstruct_or_count() {
    for fills in [
        Val::Arr(vec![]),
        hidden_paint("visible", Val::Bool(false)),
        hidden_paint("opacity", Val::Float(0.0)),
    ] {
        // A vertex style would be counted if the network were reconstructed.
        let mut blob = square_blob(2, true);
        blob[12..16].copy_from_slice(&2_u32.to_le_bytes());
        let mut doc = network_doc(blob, None);
        with_node_fills(&mut doc, fills);
        let rendered = Renderer::new().render(&doc, id(1), &viewport());
        assert_eq!(rendered.stats.unsupported_paints, 0);
        assert_eq!(pixel(rendered.pixmap.data_as_u8_slice(), 20, 20)[3], 0);
    }
}

#[test]
fn large_networks_with_inside_strokes_complete_without_a_fill() {
    // Many repeated region loops: the closure scan must not build a path per loop.
    let mut blob = Vec::new();
    for n in [3_u32, 3, 1] {
        blob.extend(n.to_le_bytes());
    }
    for (x, y) in [(0.0_f32, 0.0_f32), (10.0, 0.0), (0.0, 10.0)] {
        blob.extend(0_u32.to_le_bytes());
        blob.extend(x.to_le_bytes());
        blob.extend(y.to_le_bytes());
    }
    for (start, end) in [(0_u32, 1_u32), (1, 2), (2, 0)] {
        for n in [0, start, 0, 0, end, 0, 0] {
            blob.extend(n.to_le_bytes());
        }
    }
    let loops = 50_000_u32;
    for n in [1_u32, loops] {
        blob.extend(n.to_le_bytes());
    }
    for _ in 0..loops {
        for n in [3_u32, 0, 1, 2] {
            blob.extend(n.to_le_bytes());
        }
    }
    let mut doc = network_doc(blob, None);
    with_node_fills(&mut doc, Val::Arr(vec![]));
    let mut nodes = doc.nodes.clone();
    let data = &mut nodes.iter_mut().find(|n| n.id == id(2)).unwrap().data;
    data.set("strokePaints", paint(0.0, 0.0, 1.0));
    data.set("strokeWeight", Val::Float(2.0));
    data.set("strokeAlign", Val::Str("INSIDE".into()));
    doc.set_nodes(nodes).unwrap();
    let rendered = Renderer::new().render(&doc, id(1), &viewport());
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
