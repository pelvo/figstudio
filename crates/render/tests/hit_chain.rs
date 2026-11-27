mod common;

use common::*;
use figstudio_fig::{Document, Guid, Val};
use figstudio_render::{HitOptions, hit_chain, hit_chain_with_options, hit_test, kurbo::Point};

const INTERACTIVE: HitOptions = HitOptions {
    tolerance: 4.0,
    skip_locked: true,
};

#[test]
fn chain_preserves_ancestors_and_deepest_contract() {
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 1, "FRAME"),
        node(3, 2, "GROUP"),
        node(4, 3, "TEXT"),
    ]);
    let point = Point::new(10.0, 10.0);
    assert_eq!(hit_chain(&doc, id(1), point), [id(2), id(3), id(4)]);
    assert_eq!(hit_test(&doc, id(1), point), Some(id(4)));
    assert!(hit_chain(&doc, id(1), Point::new(100.0, 100.0)).is_empty());
}

#[test]
fn sections_are_not_selectable_but_instances_are_leaves() {
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 1, "SECTION"),
        node(3, 2, "INSTANCE"),
        node(4, 3, "TEXT"),
    ]);
    assert_eq!(hit_chain(&doc, id(1), Point::new(10.0, 10.0)), [id(3)]);
    let doc = document(vec![node(1, 0, "CANVAS"), node(2, 1, "SECTION")]);
    assert!(hit_chain(&doc, id(1), Point::new(10.0, 10.0)).is_empty());
}

#[test]
fn hit_order_visibility_and_masks_match_painting() {
    let mut hidden = node(5, 2, "RECTANGLE");
    hidden.data.set("visible", Val::Bool(false));
    let mut mask = node(6, 2, "RECTANGLE");
    mask.data.set("mask", Val::Bool(true));
    let mut nodes = vec![
        node(1, 0, "CANVAS"),
        node(2, 1, "GROUP"),
        node(3, 2, "RECTANGLE"),
        node(4, 2, "RECTANGLE"),
        hidden,
        mask,
    ];
    let point = Point::new(10.0, 10.0);
    assert_eq!(
        hit_chain(&document(nodes.clone()), id(1), point),
        [id(2), id(4)]
    );
    nodes[1].data.set("stackReverseZIndex", Val::Bool(true));
    assert_eq!(hit_chain(&document(nodes), id(1), point), [id(2), id(3)]);
}

#[test]
fn clip_and_overflow_use_the_same_transformed_walk() {
    let mut frame = node(2, 1, "FRAME");
    let mut child = node(3, 2, "TEXT");
    child.data.set(
        "transform",
        obj(&[
            ("m00", Val::Float(1.0)),
            ("m11", Val::Float(1.0)),
            ("m02", Val::Float(50.0)),
        ]),
    );
    let point = Point::new(60.0, 10.0);
    let doc = document(vec![node(1, 0, "CANVAS"), frame.clone(), child.clone()]);
    assert_eq!(hit_chain(&doc, id(1), point), [id(2), id(3)]);
    frame.data.set("frameMaskDisabled", Val::Bool(false));
    let doc = document(vec![node(1, 0, "CANVAS"), frame, child]);
    assert!(hit_chain(&doc, id(1), point).is_empty());
}

#[test]
fn depth_is_bounded_on_hostile_nesting() {
    let mut groups = vec![node(1, 0, "CANVAS")];
    groups.extend((2..200).map(|i| node(i, i - 1, "GROUP")));
    let group_doc = document(groups);
    let point = Point::new(10.0, 10.0);
    // Raw hit_chain preserves the legacy rectangular box contract on nested GROUPs.
    assert_eq!(hit_chain(&group_doc, id(1), point).len(), 128);
    // Interactive geometry picking terminates cleanly without self-hitting empty groups.
    assert!(hit_chain_with_options(&group_doc, id(1), point, INTERACTIVE).is_empty());

    let mut frames = vec![node(1, 0, "CANVAS")];
    frames.extend((2..200).map(|i| node(i, i - 1, "FRAME")));
    assert_eq!(
        hit_chain_with_options(&document(frames), id(1), point, INTERACTIVE).len(),
        128
    );
}

#[test]
fn structure_chain_stops_at_the_instance() {
    let doc = Document::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    let instance = Guid(1, 38);
    let bounds = figstudio_render::node_bounds(&doc, instance).unwrap();
    let chain = hit_chain(&doc, doc.page_of(instance).unwrap(), bounds.center());
    assert_eq!(chain, [Guid(1, 4), instance]);
    assert_eq!(
        hit_test(&doc, doc.page_of(instance).unwrap(), bounds.center()),
        Some(instance)
    );
}

fn poly_blob(points: &[(f32, f32)]) -> Vec<u8> {
    let mut b = Vec::new();
    for (i, &(x, y)) in points.iter().enumerate() {
        b.push(if i == 0 { 1 } else { 2 });
        b.extend(x.to_le_bytes());
        b.extend(y.to_le_bytes());
    }
    b.push(0);
    b
}

fn rect_cmds(out: &mut Vec<u8>, x0: f32, y0: f32, x1: f32, y1: f32) {
    for (cmd, x, y) in [(1u8, x0, y0), (2, x1, y0), (2, x1, y1), (2, x0, y1)] {
        out.push(cmd);
        out.extend(x.to_le_bytes());
        out.extend(y.to_le_bytes());
    }
    out.push(0);
}

fn square_network(closed: bool, w: f32, h: f32) -> Vec<u8> {
    let mut b = Vec::new();
    let u = |b: &mut Vec<u8>, v: u32| b.extend(v.to_le_bytes());
    for v in [4, 4, u32::from(closed)] {
        u(&mut b, v);
    }
    for (x, y) in [(0.0f32, 0.0f32), (w, 0.0), (w, h), (0.0, h)] {
        u(&mut b, 0);
        b.extend(x.to_le_bytes());
        b.extend(y.to_le_bytes());
    }
    for s in 0..4u32 {
        u(&mut b, 0);
        u(&mut b, s);
        b.extend([0u8; 8]);
        u(&mut b, (s + 1) % 4);
        b.extend([0u8; 8]);
    }
    if closed {
        for v in [0, 1, 4, 0, 1, 2, 3] {
            u(&mut b, v);
        }
    }
    b
}

fn connector_part(local: u32, fields: &[(&str, Val)]) -> Val {
    let mut v = obj(fields);
    let guid = Guid(40_000_000, local).to_val();
    v.set("guidPath", obj(&[("guids", Val::Arr(vec![guid]))]));
    v
}

#[test]
fn bent_connector_hits_stroke_and_label_only_and_passes_empty_bbox_through() {
    let mut doc = document(vec![]);
    let top = doc.blobs.len() as i64;
    doc.blobs.push(poly_blob(&[
        (0.0, 0.0),
        (200.0, 0.0),
        (200.0, 2.0),
        (0.0, 2.0),
    ]));
    let right = doc.blobs.len() as i64;
    doc.blobs.push(poly_blob(&[
        (198.0, 0.0),
        (200.0, 0.0),
        (200.0, 140.0),
        (198.0, 140.0),
    ]));
    let geom = Val::Arr(vec![
        obj(&[("commandsBlob", Val::Int(top))]),
        obj(&[("commandsBlob", Val::Int(right))]),
    ]);

    let mut frame = node(2, 1, "FRAME");
    frame.data.set("size", size(200.0, 140.0));
    let mut child = node(3, 2, "TEXT");
    child.data.set("size", size(80.0, 40.0));
    child.data.set(
        "transform",
        obj(&[("m02", Val::Float(40.0)), ("m12", Val::Float(50.0))]),
    );

    let mut conn = node(4, 1, "CONNECTOR");
    conn.data.set("size", size(200.0, 140.0));
    conn.data.set(
        "derivedImmutableFrameData",
        obj(&[(
            "overrides",
            Val::Arr(vec![
                connector_part(0, &[("size", size(200.0, 140.0)), ("strokeGeometry", geom)]),
                connector_part(
                    2,
                    &[
                        ("size", size(30.0, 16.0)),
                        (
                            "transform",
                            obj(&[("m02", Val::Float(90.0)), ("m12", Val::Float(-8.0))]),
                        ),
                    ],
                ),
            ]),
        )]),
    );
    conn.data.set(
        "nodeGenerationData",
        obj(&[(
            "overrides",
            Val::Arr(vec![
                connector_part(
                    0,
                    &[
                        ("strokePaints", paint(0.0, 0.0, 0.0)),
                        ("strokeWeight", Val::Float(2.0)),
                    ],
                ),
                connector_part(2, &[("fillPaints", paint(1.0, 1.0, 1.0))]),
            ]),
        )]),
    );
    doc.set_nodes(vec![node(1, 0, "CANVAS"), frame, child, conn])
        .unwrap();

    // Raw hit_test/hit_chain keep the legacy rectangular box contract on the same fixture.
    assert_eq!(hit_chain(&doc, id(1), Point::new(70.0, 70.0)), [id(4)]);
    assert_eq!(hit_test(&doc, id(1), Point::new(70.0, 70.0)), Some(id(4)));
    assert!(hit_chain(&doc, id(1), Point::new(105.0, -4.0)).is_empty());

    // Options-bearing geometry picking passes off-stroke interior through to FRAME -> TEXT,
    // hits the stroke even with exact tolerance = 0.0, and hits the overflowing label pill.
    for opts in [HitOptions::default(), INTERACTIVE] {
        assert_eq!(
            hit_chain_with_options(&doc, id(1), Point::new(70.0, 70.0), opts),
            [id(2), id(3)]
        );
        assert_eq!(
            hit_chain_with_options(&doc, id(1), Point::new(50.0, 1.0), opts),
            [id(4)]
        );
        assert_eq!(
            hit_chain_with_options(&doc, id(1), Point::new(199.0, 80.0), opts),
            [id(4)]
        );
        assert_eq!(
            hit_chain_with_options(&doc, id(1), Point::new(105.0, -4.0), opts),
            [id(4)]
        );
    }
}

#[test]
fn vector_network_and_line_respect_exact_tolerance_caps_anisotropy_and_zero_weight() {
    // Open L-shaped vector network (no strokeGeometry): (0,0) -> (100,0) -> (100,100).
    let mut net = Vec::new();
    let u = |b: &mut Vec<u8>, n: u32| b.extend(n.to_le_bytes());
    for n in [3, 2, 0] {
        u(&mut net, n);
    }
    for (x, y) in [(0.0_f32, 0.0_f32), (100.0, 0.0), (100.0, 100.0)] {
        u(&mut net, 0);
        net.extend(x.to_le_bytes());
        net.extend(y.to_le_bytes());
    }
    for (a, b) in [(0, 1), (1, 2)] {
        for n in [0, a, 0, 0, b, 0, 0] {
            u(&mut net, n);
        }
    }

    let mut frame = node(2, 1, "FRAME");
    frame.data.set("size", size(160.0, 160.0));
    let mut vec_node = node(3, 1, "VECTOR");
    vec_node.data.set("size", size(100.0, 100.0));
    vec_node.data.set("fillPaints", Val::Arr(vec![]));
    vec_node.data.set("strokePaints", paint(0.0, 0.0, 0.0));
    vec_node.data.set("strokeWeight", Val::Float(1.0));
    vec_node.data.set(
        "vectorData",
        obj(&[
            ("vectorNetworkBlob", Val::Int(0)),
            ("normalizedSize", size(100.0, 100.0)),
        ]),
    );
    let mut thin_line = node(4, 1, "LINE");
    thin_line.data.set("size", size(80.0, 0.0));
    thin_line.data.set(
        "transform",
        obj(&[("m02", Val::Float(10.0)), ("m12", Val::Float(60.0))]),
    );
    thin_line.data.set("strokePaints", paint(0.0, 0.0, 0.0));
    thin_line.data.set("strokeWeight", Val::Float(1.0));

    // Zero-weight LINE and zero-weight network VECTOR must never phantom-hit under tolerance.
    let mut zero_line = node(5, 1, "LINE");
    zero_line.data.set("size", size(80.0, 0.0));
    zero_line.data.set(
        "transform",
        obj(&[("m02", Val::Float(10.0)), ("m12", Val::Float(80.0))]),
    );
    zero_line.data.set("strokePaints", paint(0.0, 0.0, 0.0));
    zero_line.data.set("strokeWeight", Val::Float(0.0));

    let mut zero_vec = vec_node.clone();
    zero_vec.id = id(6);
    zero_vec.data.set("guid", id(6).to_val());
    zero_vec.data.set(
        "transform",
        obj(&[("m02", Val::Float(0.0)), ("m12", Val::Float(95.0))]),
    );
    zero_vec.data.set("strokeWeight", Val::Float(0.0));

    // Thick 20px butt-capped LINE must end flush at x = w (no semicircular overhang past end).
    let mut butt_line = node(7, 1, "LINE");
    butt_line.data.set("size", size(50.0, 0.0));
    butt_line.data.set(
        "transform",
        obj(&[("m02", Val::Float(10.0)), ("m12", Val::Float(125.0))]),
    );
    butt_line.data.set("strokePaints", paint(0.0, 0.0, 0.0));
    butt_line.data.set("strokeWeight", Val::Float(20.0));

    // Anisotropically scaled 1px LINE (scale(10, 1)) must retain 4u slop along its thin y axis.
    let mut stretched_line = node(8, 1, "LINE");
    stretched_line.data.set("size", size(10.0, 0.0));
    stretched_line.data.set(
        "transform",
        obj(&[
            ("m00", Val::Float(10.0)),
            ("m11", Val::Float(1.0)),
            ("m02", Val::Float(10.0)),
            ("m12", Val::Float(150.0)),
        ]),
    );
    stretched_line
        .data
        .set("strokePaints", paint(0.0, 0.0, 0.0));
    stretched_line.data.set("strokeWeight", Val::Float(1.0));

    let mut doc = document(vec![
        node(1, 0, "CANVAS"),
        frame,
        vec_node,
        thin_line,
        zero_line,
        zero_vec,
        butt_line,
        stretched_line,
    ]);
    doc.blobs = vec![net];

    // Raw hit_chain preserves rectangular box hits on the same fixture.
    assert_eq!(hit_chain(&doc, id(1), Point::new(30.0, 30.0)), [id(3)]);
    assert_eq!(hit_test(&doc, id(1), Point::new(30.0, 30.0)), Some(id(3)));
    assert_eq!(hit_chain(&doc, id(1), Point::new(50.0, 80.0)), [id(5)]);

    // Exact geometry (HitOptions::default(), tol=0) passes off-stroke interior through to FRAME,
    // misses 2u away from a 1px stroke, and hits directly on 0.25u.
    let exact = HitOptions::default();
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(30.0, 30.0), exact),
        [id(2)]
    );
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(50.0, 2.0), exact),
        [id(2)]
    );
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(50.0, 0.25), exact),
        [id(3)]
    );
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(50.0, 62.0), exact),
        [id(2)]
    );
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(50.0, 60.25), exact),
        [id(4)]
    );
    // Interactive tolerance (4.0) catches thin strokes within 4u, misses beyond.
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(50.0, 2.0), INTERACTIVE),
        [id(3)]
    );
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(50.0, 62.0), INTERACTIVE),
        [id(4)]
    );
    assert_eq!(
        hit_chain_with_options(
            &doc,
            id(1),
            Point::new(50.0, 62.5),
            HitOptions {
                tolerance: 1.0,
                skip_locked: true,
            }
        ),
        [id(2)]
    );
    // Zero-weight LINE and VECTOR never hit, even with interactive tolerance.
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(50.0, 80.0), INTERACTIVE),
        [id(2)]
    );
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(50.0, 95.0), INTERACTIVE),
        [id(2)]
    );
    // Thick 20px Cap::Butt line (x=10..60, y=115..135) hits inside end, misses 2u past x=60.
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(59.0, 125.0), INTERACTIVE),
        [id(7)]
    );
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(62.0, 125.0), INTERACTIVE),
        [id(2)]
    );
    // Anisotropically scaled thin line retains click slop along its 1px vertical axis.
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(50.0, 153.0), INTERACTIVE),
        [id(8)]
    );
}

#[test]
fn paint_styles_aligned_strokes_zero_opacity_network_and_clips_match_rendering() {
    // 1. Style nodes: local guid visible style, assetRef visible style, and empty style.
    let mut visible_style = node(20, 0, "RECTANGLE");
    visible_style.data.set("styleType", Val::Str("FILL".into()));
    visible_style.data.set("fillPaints", paint(0.0, 0.0, 0.0));

    let mut asset_style = node(21, 0, "RECTANGLE");
    asset_style.data.set("styleType", Val::Str("FILL".into()));
    asset_style.data.set("key", Val::Str("ink".into()));
    asset_style.data.set("version", Val::Str("v1".into()));
    asset_style.data.set("fillPaints", paint(0.0, 0.0, 0.0));

    let mut empty_style = node(22, 0, "RECTANGLE");
    empty_style.data.set("styleType", Val::Str("FILL".into()));
    empty_style.data.set("fillPaints", Val::Arr(vec![]));

    let mut bg = node(2, 1, "FRAME");
    bg.data.set("size", size(260.0, 200.0));

    // Styled LINE whose stale cached strokePaints is empty must hit via styleIdForStrokeFill.
    let mut styled_line = node(3, 1, "LINE");
    styled_line.data.set("size", size(60.0, 0.0));
    styled_line.data.set(
        "transform",
        obj(&[("m02", Val::Float(10.0)), ("m12", Val::Float(20.0))]),
    );
    styled_line.data.set("strokePaints", Val::Arr(vec![]));
    styled_line
        .data
        .set("styleIdForStrokeFill", obj(&[("guid", id(20).to_val())]));
    styled_line.data.set("strokeWeight", Val::Float(2.0));

    // Asset-styled LINE must hit too.
    let mut asset_line = node(4, 1, "LINE");
    asset_line.data.set("size", size(60.0, 0.0));
    asset_line.data.set(
        "transform",
        obj(&[("m02", Val::Float(10.0)), ("m12", Val::Float(40.0))]),
    );
    asset_line.data.set("strokePaints", Val::Arr(vec![]));
    asset_line.data.set(
        "styleIdForStrokeFill",
        obj(&[(
            "assetRef",
            obj(&[
                ("key", Val::Str("ink".into())),
                ("version", Val::Str("v1".into())),
            ]),
        )]),
    );
    asset_line.data.set("strokeWeight", Val::Float(2.0));

    // LINE whose style was cleared to empty overrides stale cached strokePaints and must NOT hit.
    let mut cleared_line = node(5, 1, "LINE");
    cleared_line.data.set("size", size(60.0, 0.0));
    cleared_line.data.set(
        "transform",
        obj(&[("m02", Val::Float(10.0)), ("m12", Val::Float(60.0))]),
    );
    cleared_line.data.set("strokePaints", paint(0.0, 0.0, 0.0));
    cleared_line
        .data
        .set("styleIdForStrokeFill", obj(&[("guid", id(22).to_val())]));
    cleared_line.data.set("strokeWeight", Val::Float(2.0));

    // 2. Network-backed VECTOR with opacity: 0.0 enabled fill (no fillGeometry) must still hit.
    let sq_net = square_network(true, 40.0, 40.0);
    let mut zero_alpha = paint(0.0, 0.0, 0.0).items().to_vec();
    zero_alpha[0].set("opacity", Val::Float(0.0));
    let mut net_fill = node(6, 1, "VECTOR");
    net_fill.data.set("size", size(40.0, 40.0));
    net_fill.data.set(
        "transform",
        obj(&[("m02", Val::Float(100.0)), ("m12", Val::Float(10.0))]),
    );
    net_fill.data.set("fillPaints", Val::Arr(zero_alpha));
    net_fill
        .data
        .set("vectorData", obj(&[("vectorNetworkBlob", Val::Int(0))]));

    // 3. Closed VECTOR with 20px stored OUTSIDE stroke (2x centered band -20..60 minus 20..20)
    // and another with 20px INSIDE stroke.
    let mut fill_blob = Vec::new();
    rect_cmds(&mut fill_blob, 0.0, 0.0, 40.0, 40.0);
    let mut band_blob = Vec::new();
    rect_cmds(&mut band_blob, -20.0, -20.0, 60.0, 60.0);
    rect_cmds(&mut band_blob, 20.0, 20.0, 20.0, 20.0);
    let fill_geom = Val::Arr(vec![obj(&[
        ("commandsBlob", Val::Int(1)),
        ("windingRule", Val::Str("NONZERO".into())),
    ])]);
    let stroke_geom = Val::Arr(vec![obj(&[
        ("commandsBlob", Val::Int(2)),
        ("windingRule", Val::Str("ODD".into())),
    ])]);

    let make_aligned = |n: u32, x: f64, y: f64, align: &str| {
        let mut v = node(n, 1, "VECTOR");
        v.data.set("size", size(40.0, 40.0));
        v.data.set(
            "transform",
            obj(&[("m02", Val::Float(x)), ("m12", Val::Float(y))]),
        );
        v.data.set("fillPaints", Val::Arr(vec![]));
        v.data.set("strokePaints", paint(0.0, 0.0, 0.0));
        v.data.set("strokeWeight", Val::Float(20.0));
        v.data.set("strokeAlign", Val::Str(align.into()));
        v.data.set("fillGeometry", fill_geom.clone());
        v.data.set("strokeGeometry", stroke_geom.clone());
        v.data
            .set("vectorData", obj(&[("vectorNetworkBlob", Val::Int(0))]));
        v
    };
    let outside_vec = make_aligned(7, 180.0, 30.0, "OUTSIDE");
    let inside_vec = make_aligned(8, 180.0, 120.0, "INSIDE");

    // 4. Clipping FRAME with a completely clipped-out child LINE at y = -2.0.
    let mut clip_frame = node(9, 1, "FRAME");
    clip_frame.data.set("size", size(80.0, 60.0));
    clip_frame.data.set(
        "transform",
        obj(&[("m02", Val::Float(10.0)), ("m12", Val::Float(110.0))]),
    );
    clip_frame.data.set("frameMaskDisabled", Val::Bool(false));
    let mut clipped_line = node(10, 9, "LINE");
    clipped_line.data.set("size", size(80.0, 0.0));
    clipped_line.data.set(
        "transform",
        obj(&[("m02", Val::Float(0.0)), ("m12", Val::Float(-2.0))]),
    );
    clipped_line.data.set("strokePaints", paint(0.0, 0.0, 0.0));
    clipped_line.data.set("strokeWeight", Val::Float(1.0));

    let mut doc = document(vec![
        node(1, 0, "CANVAS"),
        visible_style,
        asset_style,
        empty_style,
        bg,
        styled_line,
        asset_line,
        cleared_line,
        net_fill,
        outside_vec,
        inside_vec,
        clip_frame,
        clipped_line,
    ]);
    doc.blobs = vec![sq_net, fill_blob, band_blob];

    // Style resolution overrides stale cached paint arrays.
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(40.0, 20.0), INTERACTIVE),
        [id(3)]
    );
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(40.0, 40.0), INTERACTIVE),
        [id(4)]
    );
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(40.0, 60.0), INTERACTIVE),
        [id(2)]
    );

    // 0%-opacity enabled fill on vectorNetworkBlob still hits interior under geometry picking.
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(120.0, 30.0), HitOptions::default()),
        [id(6)]
    );
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(120.0, 30.0), INTERACTIVE),
        [id(6)]
    );

    // OUTSIDE 20px stroke at (180..220, 30..70): hits outside band (170, 50), passes hollow interior (190, 50).
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(170.0, 50.0), INTERACTIVE),
        [id(7)]
    );
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(190.0, 50.0), INTERACTIVE),
        [id(2)]
    );
    // INSIDE 20px stroke at (180..220, 120..160): hits inside band (190, 140), misses outside (170, 140).
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(190.0, 140.0), INTERACTIVE),
        [id(8)]
    );
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(170.0, 140.0), INTERACTIVE),
        [id(2)]
    );

    // Clipped child LINE at y = -2 inside clipping frame must not steal clicks at 100% or 25% zoom.
    assert_eq!(
        hit_chain_with_options(&doc, id(1), Point::new(50.0, 111.0), INTERACTIVE),
        [id(9)]
    );
    assert_eq!(
        hit_chain_with_options(
            &doc,
            id(1),
            Point::new(50.0, 120.0),
            HitOptions {
                tolerance: 16.0,
                skip_locked: true,
            }
        ),
        [id(9)]
    );
}

#[test]
fn locked_layers_preserve_raw_hit_test_and_pass_through_interactive_options() {
    let mut bg = node(2, 1, "RECTANGLE");
    bg.data.set("size", size(100.0, 100.0));
    bg.data.set("fillPaints", Val::Arr(vec![])); // Empty-paint primitive keeps baseline box hit.

    let mut locked = node(3, 1, "RECTANGLE");
    locked.data.set("size", size(100.0, 100.0));
    locked.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    locked.data.set("locked", Val::Bool(true));

    let doc = document(vec![node(1, 0, "CANVAS"), bg, locked]);
    let center = Point::new(50.0, 50.0);
    // Raw public hit_test / hit_chain preserves exact visibility without skipping locked layers.
    assert_eq!(hit_test(&doc, id(1), center), Some(id(3)));
    assert_eq!(hit_chain(&doc, id(1), center), [id(3)]);
    // Interactive canvas options skip locked layers and still hit empty-paint primitive shapes.
    assert_eq!(
        hit_chain_with_options(&doc, id(1), center, INTERACTIVE),
        [id(2)]
    );
}

#[test]
fn raw_box_vs_interactive_geometry_on_same_synthetic_fixture() {
    let mut bg = node(2, 1, "FRAME");
    bg.data.set("size", size(220.0, 140.0));
    let mut text = node(3, 2, "TEXT");
    text.data.set("size", size(80.0, 40.0));
    text.data.set(
        "transform",
        obj(&[("m02", Val::Float(20.0)), ("m12", Val::Float(40.0))]),
    );

    // Bent 2px stroke-only VECTOR covering (0..120, 0..100); empty interior overlaps TEXT.
    let mut arrow_doc = document(vec![]);
    let top = arrow_doc.blobs.len() as i64;
    arrow_doc.blobs.push(poly_blob(&[
        (0.0, 0.0),
        (120.0, 0.0),
        (120.0, 2.0),
        (0.0, 2.0),
    ]));
    let right = arrow_doc.blobs.len() as i64;
    arrow_doc.blobs.push(poly_blob(&[
        (118.0, 0.0),
        (120.0, 0.0),
        (120.0, 100.0),
        (118.0, 100.0),
    ]));
    let mut arrow = node(4, 1, "VECTOR");
    arrow.data.set("size", size(120.0, 100.0));
    arrow.data.set("fillPaints", Val::Arr(vec![]));
    arrow.data.set("strokePaints", paint(0.0, 0.0, 0.0));
    arrow.data.set("strokeWeight", Val::Float(2.0));
    arrow.data.set(
        "strokeGeometry",
        Val::Arr(vec![
            obj(&[("commandsBlob", Val::Int(top))]),
            obj(&[("commandsBlob", Val::Int(right))]),
        ]),
    );

    // Nonpainting GROUP and resizeToFit FRAME overlapping bg at x = 130..170 and 175..215.
    let mut group = node(5, 1, "GROUP");
    group.data.set("size", size(40.0, 40.0));
    group.data.set(
        "transform",
        obj(&[("m02", Val::Float(130.0)), ("m12", Val::Float(10.0))]),
    );
    let mut rtf = node(6, 1, "FRAME");
    rtf.data.set("size", size(40.0, 40.0));
    rtf.data.set("resizeToFit", Val::Bool(true));
    rtf.data.set(
        "transform",
        obj(&[("m02", Val::Float(175.0)), ("m12", Val::Float(10.0))]),
    );

    // Locked solid RECTANGLE overlapping bg at x = 130..210, y = 70..120.
    let mut locked = node(7, 1, "RECTANGLE");
    locked.data.set("size", size(80.0, 50.0));
    locked.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    locked.data.set("locked", Val::Bool(true));
    locked.data.set(
        "transform",
        obj(&[("m02", Val::Float(130.0)), ("m12", Val::Float(70.0))]),
    );

    arrow_doc
        .set_nodes(vec![
            node(1, 0, "CANVAS"),
            bg,
            text,
            arrow,
            group,
            rtf,
            locked,
        ])
        .unwrap();

    let off_stroke = Point::new(50.0, 55.0);
    let on_stroke = Point::new(50.0, 1.0);
    let near_stroke = Point::new(50.0, 4.0);
    let in_group = Point::new(150.0, 30.0);
    let in_rtf = Point::new(195.0, 30.0);
    let in_locked = Point::new(160.0, 95.0);

    // Raw hit_test/hit_chain always perform rectangular box picks without skipping locked layers.
    assert_eq!(hit_chain(&arrow_doc, id(1), off_stroke), [id(4)]);
    assert_eq!(hit_test(&arrow_doc, id(1), off_stroke), Some(id(4)));
    assert_eq!(hit_chain(&arrow_doc, id(1), in_group), [id(5)]);
    assert_eq!(hit_chain(&arrow_doc, id(1), in_rtf), [id(6)]);
    assert_eq!(hit_test(&arrow_doc, id(1), in_locked), Some(id(7)));

    // Exact geometry (HitOptions::default()) passes hollow regions through, hits exact stroke,
    // misses outside exact stroke width, and keeps locked layers selectable.
    let exact = HitOptions::default();
    assert_eq!(
        hit_chain_with_options(&arrow_doc, id(1), off_stroke, exact),
        [id(2), id(3)]
    );
    assert_eq!(
        hit_chain_with_options(&arrow_doc, id(1), on_stroke, exact),
        [id(4)]
    );
    assert_eq!(
        hit_chain_with_options(&arrow_doc, id(1), near_stroke, exact),
        [id(2)]
    );
    assert_eq!(
        hit_chain_with_options(&arrow_doc, id(1), in_group, exact),
        [id(2)]
    );
    assert_eq!(
        hit_chain_with_options(&arrow_doc, id(1), in_rtf, exact),
        [id(2)]
    );
    assert_eq!(
        hit_chain_with_options(&arrow_doc, id(1), in_locked, exact),
        [id(7)]
    );

    // Interactive canvas options add stroke tolerance and skip locked layers.
    assert_eq!(
        hit_chain_with_options(&arrow_doc, id(1), off_stroke, INTERACTIVE),
        [id(2), id(3)]
    );
    assert_eq!(
        hit_chain_with_options(&arrow_doc, id(1), on_stroke, INTERACTIVE),
        [id(4)]
    );
    assert_eq!(
        hit_chain_with_options(&arrow_doc, id(1), near_stroke, INTERACTIVE),
        [id(4)]
    );
    assert_eq!(
        hit_chain_with_options(&arrow_doc, id(1), in_group, INTERACTIVE),
        [id(2)]
    );
    assert_eq!(
        hit_chain_with_options(&arrow_doc, id(1), in_rtf, INTERACTIVE),
        [id(2)]
    );
    assert_eq!(
        hit_chain_with_options(&arrow_doc, id(1), in_locked, INTERACTIVE),
        [id(2)]
    );
}

#[test]
fn vector_fixture_hits_painted_regions_and_misses_empty_corners() {
    let doc = Document::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/vector.fig"),
    )
    .unwrap();
    let page = doc.pages[0];
    let vec_id = doc.nodes.iter().find(|n| n.kind == "VECTOR").unwrap().id;
    let frame_id = doc.nodes.iter().find(|n| n.kind == "FRAME").unwrap().id;
    let b = figstudio_render::node_bounds(&doc, vec_id).unwrap();
    let corner = Point::new(b.x0 + 1.0, b.y0 + 1.0);
    // Raw hit_test preserves rectangular box hits on the vector's empty bounding-box corner.
    assert_eq!(hit_test(&doc, page, b.center()), Some(vec_id));
    assert_eq!(hit_test(&doc, page, corner), Some(vec_id));
    // Options-bearing geometry picking hits painted regions and passes empty corners through.
    for opts in [HitOptions::default(), INTERACTIVE] {
        assert_eq!(
            hit_chain_with_options(&doc, page, b.center(), opts).last(),
            Some(&vec_id)
        );
        assert_eq!(
            hit_chain_with_options(&doc, page, corner, opts).last(),
            Some(&frame_id)
        );
    }
}
