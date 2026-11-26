//! Render regressions for PR #90 Findings 1-2: instance visual inheritance through resize,
//! corner radii, Auto Layout stretch, top-level swap, and detach.

use std::path::PathBuf;

use figstudio_fig::edit::geometry::{encode_path_commands, make_path_entry, rounded_rect_commands};
use figstudio_fig::edit::{EditOp, History, Txn, execute_edit_ops};
use figstudio_fig::{Document, Guid, Val};
use figstudio_render::kurbo::Affine;
use figstudio_render::{Renderer, Viewport};

fn empty_page_doc() -> (Document, Guid, History) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let mut doc = Document::open(path).unwrap();
    let mut history = History::new(30);
    let page = execute_edit_ops(
        &mut doc,
        &mut history,
        &[EditOp::CreatePage {
            id: None,
            name: "Components".into(),
        }],
        None,
        None,
        false,
    )
    .unwrap()
    .0
    .created_ids[0];
    (doc, page, history)
}

fn run(doc: &mut Document, history: &mut History, ops: &[EditOp]) -> Vec<Guid> {
    execute_edit_ops(doc, history, ops, None, None, false)
        .unwrap()
        .0
        .created_ids
}

fn render(doc: &Document, page: Guid, w: u16, h: u16) -> Vec<u8> {
    let mut renderer = Renderer::new();
    let frame = renderer.render(
        doc,
        page,
        &Viewport {
            width: w,
            height: h,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    frame.pixmap.data_as_u8_slice().to_vec()
}

fn pixel(px: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * w + x) * 4;
    [px[i], px[i + 1], px[i + 2], px[i + 3]]
}

#[test]
fn resized_and_rounded_instance_renders_updated_fill_and_inside_stroke() {
    let (mut doc, page, mut history) = empty_page_doc();

    // Place the master off-viewport so only the instance renders at (0, 0).
    let master = run(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("Card".into()),
            x: 400.0,
            y: 0.0,
            width: 100.0,
            height: 40.0,
            corner_radius: 0.0,
            fill_hex: Some("#FF0000".into()),
            count: None,
            star_inner_scale: None,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[
            EditOp::SetStroke {
                target: master,
                hex: Some("#0000FF".into()),
                weight: Some(4.0),
                align: Some("INSIDE".into()),
            },
            EditOp::CreateComponent { frame: master },
        ],
    );

    let inst = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 0.0,
            y: 0.0,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[
            EditOp::Resize {
                target: inst,
                width: 200.0,
                height: 40.0,
            },
            EditOp::SetCornerRadii {
                target: inst,
                radii: [16.0; 4],
                smoothing: None,
            },
        ],
    );

    let px = render(&doc, page, 220, 60);
    assert_eq!(
        pixel(&px, 220, 180, 20),
        [255, 0, 0, 255],
        "resized instance fill did not extend to x=180"
    );
    assert_eq!(
        pixel(&px, 220, 198, 20),
        [0, 0, 255, 255],
        "INSIDE stroke was clipped to stale 100px box instead of 200px edge"
    );
    assert_eq!(
        pixel(&px, 220, 1, 1)[3],
        0,
        "rounded instance corner at (1, 1) drew stale sharp fillGeometry"
    );
}

#[test]
fn auto_layout_stretched_instance_renders_at_stretched_width() {
    let (mut doc, page, mut history) = empty_page_doc();

    let master = run(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("Chip".into()),
            x: 400.0,
            y: 0.0,
            width: 60.0,
            height: 40.0,
            corner_radius: 0.0,
            fill_hex: Some("#00AA00".into()),
            count: None,
            star_inner_scale: None,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[EditOp::CreateComponent { frame: master }],
    );

    // Row 1 (y=0): newly instantiated component stretched via primary_fill.
    // Row 2 (y=60): imported-style instance carrying pre-baked 60x40 fillGeometry, stretched via primary_fill.
    for (y, seed_baked_fill_geom) in [(0.0, false), (60.0, true)] {
        let row = run(
            &mut doc,
            &mut history,
            &[EditOp::CreateShape {
                id: None,
                kind: "FRAME".into(),
                parent: page,
                name: Some("Row".into()),
                x: 0.0,
                y,
                width: 200.0,
                height: 40.0,
                corner_radius: 0.0,
                fill_hex: Some("#FFFFFF00".into()),
                count: None,
                star_inner_scale: None,
            }],
        )[0];
        run(
            &mut doc,
            &mut history,
            &[EditOp::SetAutoLayout {
                target: row,
                mode: "HORIZONTAL".into(),
                spacing: Some(0.0),
                counter_spacing: None,
                padding: Some([0.0, 0.0, 0.0, 0.0]),
                primary_sizing: Some("FIXED".into()),
                counter_sizing: Some("FIXED".into()),
                primary_align: None,
                counter_align: None,
                wrap: None,
            }],
        );
        let inst = run(
            &mut doc,
            &mut history,
            &[EditOp::InstantiateComponent {
                symbol: master,
                parent: row,
                x: 0.0,
                y: 0.0,
            }],
        )[0];
        if seed_baked_fill_geom {
            let mut t = Txn::new("seed imported 60x40 fillGeometry");
            let blob_idx = doc.blobs.len();
            t.add_blobs
                .push(encode_path_commands(&rounded_rect_commands(
                    60.0, 40.0, [0.0; 4], 0.0,
                )));
            t.set_field(
                inst,
                "fillGeometry",
                Val::Arr(vec![make_path_entry(blob_idx, false)]),
            );
            doc.apply_txn(&t).unwrap();
        }
        run(
            &mut doc,
            &mut history,
            &[EditOp::SetAutoLayoutChild {
                target: inst,
                primary_fill: Some(true),
                counter_stretch: None,
                positioning: None,
                min_size: None,
                max_size: None,
            }],
        );
    }

    let px = render(&doc, page, 220, 120);
    assert_eq!(
        pixel(&px, 220, 180, 20),
        [0, 170, 0, 255],
        "newly instantiated Auto Layout FILL instance stopped at old 60px width"
    );
    assert_eq!(
        pixel(&px, 220, 180, 80),
        [0, 170, 0, 255],
        "imported-style Auto Layout FILL instance kept stale 60px fillGeometry"
    );
}

#[test]
fn swapped_top_level_instance_renders_new_master_fill_and_corners_before_and_after_detach() {
    let (mut doc, page, mut history) = empty_page_doc();

    let master_a = run(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("Master A".into()),
            x: 300.0,
            y: 0.0,
            width: 100.0,
            height: 40.0,
            corner_radius: 0.0,
            fill_hex: Some("#00FF00".into()),
            count: None,
            star_inner_scale: None,
        }],
    )[0];
    let master_b = run(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("Master B".into()),
            x: 300.0,
            y: 80.0,
            width: 100.0,
            height: 40.0,
            corner_radius: 16.0,
            fill_hex: Some("#0000FF".into()),
            count: None,
            star_inner_scale: None,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[
            EditOp::CreateComponent { frame: master_a },
            EditOp::CreateComponent { frame: master_b },
        ],
    );

    let inst = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master_a,
            parent: page,
            x: 0.0,
            y: 0.0,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[EditOp::SwapInstance {
            instance: inst,
            symbol: master_b,
        }],
    );

    let px_swapped = render(&doc, page, 120, 60);
    assert_eq!(
        pixel(&px_swapped, 120, 50, 20),
        [0, 0, 255, 255],
        "swapped top-level instance still rendered master A's green fill"
    );
    assert_eq!(
        pixel(&px_swapped, 120, 1, 1)[3],
        0,
        "swapped top-level instance still rendered master A's sharp corner"
    );

    run(
        &mut doc,
        &mut history,
        &[EditOp::DetachInstance { instance: inst }],
    );
    let px_detached = render(&doc, page, 120, 60);
    assert_eq!(
        pixel(&px_detached, 120, 50, 20),
        [0, 0, 255, 255],
        "detached swapped instance did not render master B's blue fill"
    );
    assert_eq!(
        pixel(&px_detached, 120, 1, 1)[3],
        0,
        "detached swapped instance did not render master B's rounded corner"
    );
}

#[test]
fn imported_instance_with_prebaked_fill_geometry_updates_footprint_on_resize_and_corner_radii() {
    let (mut doc, page, mut history) = empty_page_doc();

    // Create a component master off-viewport, instantiate it, and seed pre-baked 100x40 sharp
    // `fillGeometry` (matching an imported Figma `.fig` INSTANCE).
    let master = run(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("Imported Card".into()),
            x: 400.0,
            y: 0.0,
            width: 100.0,
            height: 40.0,
            corner_radius: 0.0,
            fill_hex: Some("#FF0000".into()),
            count: None,
            star_inner_scale: None,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[
            EditOp::SetStroke {
                target: master,
                hex: Some("#0000FF".into()),
                weight: Some(4.0),
                align: Some("INSIDE".into()),
            },
            EditOp::CreateComponent { frame: master },
        ],
    );

    let seed_baked = |doc: &mut Document, inst: Guid, w: f32, h: f32| {
        let mut t = Txn::new("seed imported sharp fillGeometry");
        let blob_idx = doc.blobs.len();
        t.add_blobs
            .push(encode_path_commands(&rounded_rect_commands(
                w, h, [0.0; 4], 0.0,
            )));
        t.set_field(
            inst,
            "fillGeometry",
            Val::Arr(vec![make_path_entry(blob_idx, false)]),
        );
        doc.apply_txn(&t).unwrap();
    };

    // 1. Manual Resize (grow 100 -> 200) followed by SetCornerRadii on an imported INSTANCE.
    let inst_grow = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 0.0,
            y: 0.0,
        }],
    )[0];
    seed_baked(&mut doc, inst_grow, 100.0, 40.0);

    run(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: inst_grow,
            width: 200.0,
            height: 40.0,
        }],
    );
    let px_resized = render(&doc, page, 220, 60);
    assert_eq!(
        pixel(&px_resized, 220, 180, 20),
        [255, 0, 0, 255],
        "resized imported instance kept stale 100px fillGeometry instead of expanding to x=180"
    );
    assert_eq!(
        pixel(&px_resized, 220, 198, 20),
        [0, 0, 255, 255],
        "resized imported instance did not draw INSIDE stroke at 200px edge"
    );

    // Re-seed stale fillGeometry to verify SetCornerRadii independently drops/refreshes it too.
    seed_baked(&mut doc, inst_grow, 200.0, 40.0);
    run(
        &mut doc,
        &mut history,
        &[EditOp::SetCornerRadii {
            target: inst_grow,
            radii: [16.0; 4],
            smoothing: None,
        }],
    );
    let px_rounded = render(&doc, page, 220, 60);
    assert_eq!(
        pixel(&px_rounded, 220, 1, 1)[3],
        0,
        "rounded imported instance corner at (1, 1) still drew stale sharp fillGeometry"
    );

    // 2. Manual Resize (shrink 100 -> 60) must not leave stale fill pixels at x=80.
    let inst_shrink = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 0.0,
            y: 60.0,
        }],
    )[0];
    seed_baked(&mut doc, inst_shrink, 100.0, 40.0);
    run(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: inst_shrink,
            width: 60.0,
            height: 40.0,
        }],
    );
    let px_shrunk = render(&doc, page, 220, 120);
    assert_eq!(
        pixel(&px_shrunk, 220, 80, 80)[3],
        0,
        "shrunk imported instance still painted stale 100px fillGeometry at x=80"
    );
}
