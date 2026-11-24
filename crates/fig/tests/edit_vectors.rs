//! Live booleans on any number of layers, convert to vector, flatten, outline stroke, and
//! vertex edits. Areas are measured on the stored geometry.

use std::f64::consts::PI;
use std::path::PathBuf;

use figstudio_fig::edit::geometry::decode_path_commands;
use figstudio_fig::edit::outline::to_bez;
use figstudio_fig::edit::space::layer_bounds;
use figstudio_fig::edit::vector_net::BooleanOp;
use figstudio_fig::edit::{EditOp, History, VectorEdit, execute_edit_ops};
use figstudio_fig::{Document, Guid, Val};
use kurbo::Shape;

struct Fx {
    doc: Document,
    h: History,
    page: Guid,
}

impl Fx {
    fn new() -> Fx {
        let mut doc = Document::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
        )
        .unwrap();
        let mut h = History::new(50);
        let page = execute_edit_ops(
            &mut doc,
            &mut h,
            &[EditOp::CreatePage {
                id: None,
                name: "V".into(),
            }],
            None,
            None,
            false,
        )
        .unwrap()
        .0
        .created_ids[0];
        Fx { doc, h, page }
    }

    fn run(&mut self, ops: Vec<EditOp>) -> Result<Vec<Guid>, String> {
        execute_edit_ops(&mut self.doc, &mut self.h, &ops, None, None, false)
            .map(|r| r.0.created_ids)
            .map_err(|e| e.to_string())
    }

    fn shape(&mut self, kind: &str, x: f64, y: f64, w: f64, h: f64) -> Guid {
        let page = self.page;
        self.run(vec![EditOp::CreateShape {
            id: None,
            kind: kind.into(),
            parent: page,
            name: None,
            x,
            y,
            width: w,
            height: h,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }])
        .unwrap()[0]
    }

    /// Filled area of the layer's stored `fillGeometry`.
    fn area(&self, id: Guid) -> f64 {
        let data = &self.doc.node(id).unwrap().data;
        data.get("fillGeometry")
            .map(Val::items)
            .unwrap_or_default()
            .iter()
            .map(|e| {
                let i = e.get("commandsBlob").and_then(Val::as_u32).unwrap() as usize;
                to_bez(&decode_path_commands(&self.doc.blobs[i]).unwrap()).area()
            })
            .sum::<f64>()
            .abs()
    }

    fn boolean(&mut self, op: BooleanOp, targets: Vec<Guid>) -> Guid {
        self.run(vec![EditOp::BooleanCombine {
            operation: op,
            targets,
            name: None,
        }])
        .unwrap()[0]
    }
}

fn near(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1.0)
}

#[test]
fn every_boolean_operation_computes_the_exact_area() {
    for (op, want) in [
        (BooleanOp::Union, 17_500.0),
        (BooleanOp::Subtract, 7_500.0),
        (BooleanOp::Intersect, 2_500.0),
        (BooleanOp::Exclude, 15_000.0),
    ] {
        let mut f = Fx::new();
        let a = f.shape("RECTANGLE", 0.0, 0.0, 100.0, 100.0);
        let b = f.shape("RECTANGLE", 50.0, 50.0, 100.0, 100.0);
        let bool_id = f.boolean(op, vec![b, a]);
        assert!(
            near(f.area(bool_id), want, 1e-4),
            "{op:?}: {}",
            f.area(bool_id)
        );
        assert_eq!(
            f.doc.children(bool_id),
            &[a, b],
            "operands keep their z order"
        );
        assert_eq!(
            f.doc.node(bool_id).unwrap().data.text("booleanOperation"),
            Some(op.figma_name())
        );
    }
}

#[test]
fn booleans_keep_curves_exact_and_take_any_number_of_layers() {
    let mut f = Fx::new();
    let circles: Vec<Guid> = (0..3)
        .map(|i| f.shape("ELLIPSE", f64::from(i) * 200.0, 0.0, 100.0, 100.0))
        .collect();
    let u = f.boolean(BooleanOp::Union, circles);
    // Three disjoint circles of radius 50; Figma's own ellipse curves are cubic approximations.
    assert!(near(f.area(u), 3.0 * PI * 2500.0, 2e-3), "{}", f.area(u));
    let b = layer_bounds(&f.doc, u, f.page).unwrap();
    assert!(near(b.width(), 500.0, 1e-6) && near(b.height(), 100.0, 1e-6));
}

#[test]
fn a_boolean_follows_its_operands_and_undoes_in_one_step() {
    let mut f = Fx::new();
    let a = f.shape("RECTANGLE", 0.0, 0.0, 100.0, 100.0);
    let b = f.shape("RECTANGLE", 150.0, 0.0, 100.0, 100.0);
    let u = f.boolean(BooleanOp::Union, vec![a, b]);
    assert!(near(f.area(u), 20_000.0, 1e-4));
    let depth = f.h.undo_len();
    f.run(vec![EditOp::Move {
        target: b,
        x: -100.0,
        y: 0.0,
        relative: true,
    }])
    .unwrap();
    assert_eq!(f.h.undo_len(), depth + 1);
    assert!(near(f.area(u), 15_000.0, 1e-4), "{}", f.area(u));
    let w = layer_bounds(&f.doc, u, f.page).unwrap().width();
    assert!(near(w, 150.0, 1e-6), "the box refits: {w}");
    f.h.undo(&mut f.doc).unwrap();
    assert!(near(f.area(u), 20_000.0, 1e-4));
}

#[test]
fn a_boolean_with_text_falls_back_to_drawing_its_operands() {
    let mut f = Fx::new();
    let a = f.shape("RECTANGLE", 0.0, 0.0, 100.0, 100.0);
    let page = f.page;
    let t = f
        .run(vec![EditOp::CreateText {
            id: None,
            parent: page,
            x: 0.0,
            y: 0.0,
            characters: "Hi".into(),
            font_family: "Inter".into(),
            font_style: "Regular".into(),
            font_size: 20.0,
            fill_hex: "#000000".into(),
        }])
        .unwrap()[0];
    let u = f.boolean(BooleanOp::Union, vec![a, t]);
    assert!(f.doc.node(u).unwrap().data.get("fillGeometry").is_none());
    // Fewer than two layers, repeats and mixed parents are refused.
    assert!(
        f.run(vec![EditOp::BooleanCombine {
            operation: BooleanOp::Union,
            targets: vec![a],
            name: None
        }])
        .is_err()
    );
}

#[test]
fn convert_flatten_and_vertex_edits_produce_editable_vectors() {
    let mut f = Fx::new();
    let e = f.shape("ELLIPSE", 0.0, 0.0, 200.0, 100.0);
    f.run(vec![EditOp::ConvertToVector { target: e }]).unwrap();
    let n = f.doc.node(e).unwrap();
    assert_eq!(n.kind, "VECTOR");
    assert!(n.data.path(&["vectorData", "vectorNetworkBlob"]).is_some());
    assert!(near(f.area(e), PI * 100.0 * 50.0, 2e-3), "{}", f.area(e));

    let r = f.shape("RECTANGLE", 300.0, 0.0, 100.0, 100.0);
    f.run(vec![EditOp::ConvertToVector { target: r }]).unwrap();
    // A square's network: 4 vertices; pull vertex 2 (bottom right) out to make a pentagon-ish.
    f.run(vec![EditOp::EditVectorNetwork {
        target: r,
        edits: vec![VectorEdit::MoveVertex {
            index: 2,
            x: 150.0,
            y: 100.0,
        }],
    }])
    .unwrap();
    assert!(near(f.area(r), 12_500.0, 1e-4), "{}", f.area(r));
    // Removing a vertex breaks the loop: the region goes, the layer fills nothing.
    f.run(vec![EditOp::EditVectorNetwork {
        target: r,
        edits: vec![VectorEdit::RemoveVertex { index: 0 }],
    }])
    .unwrap();
    assert_eq!(f.area(r), 0.0);
    for bad in [
        VectorEdit::MoveVertex {
            index: 99,
            x: 0.0,
            y: 0.0,
        },
        VectorEdit::AddSegment { start: 0, end: 99 },
        VectorEdit::AddRegion {
            loops: vec![vec![0]],
            even_odd: false,
        },
    ] {
        assert!(
            f.run(vec![EditOp::EditVectorNetwork {
                target: r,
                edits: vec![bad.clone()]
            }])
            .is_err(),
            "{bad:?}"
        );
    }

    let a = f.shape("RECTANGLE", 0.0, 300.0, 100.0, 100.0);
    let b = f.shape("RECTANGLE", 50.0, 300.0, 100.0, 100.0);
    let v = f
        .run(vec![EditOp::Flatten {
            targets: vec![a, b],
            name: None,
        }])
        .unwrap()[0];
    assert!(f.doc.node(a).is_none() && f.doc.node(b).is_none());
    assert_eq!(f.doc.node(v).unwrap().kind, "VECTOR");
    assert!(near(f.area(v), 15_000.0, 1e-4), "{}", f.area(v));
    let reopened = Document::from_bytes(&f.doc.to_bytes().unwrap()).unwrap();
    assert_eq!(reopened.node(v).unwrap().kind, "VECTOR");
}

#[test]
fn outline_stroke_makes_the_stroke_band_a_vector_above_the_layer() {
    let mut f = Fx::new();
    let r = f.shape("RECTANGLE", 0.0, 0.0, 100.0, 50.0);
    let set = |align: &str| {
        serde_json::from_value::<EditOp>(serde_json::json!({
            "op": "set_stroke_style", "target": r.to_string(), "weight": 4, "align": align
        }))
        .unwrap()
    };
    f.run(vec![
        serde_json::from_value(serde_json::json!({
            "op": "add_paint", "target": r.to_string(), "slot": "stroke",
            "paint": { "type": "solid", "hex": "#FF0000" }
        }))
        .unwrap(),
        set("INSIDE"),
    ])
    .unwrap();
    let v = f.run(vec![EditOp::OutlineStroke { target: r }]).unwrap()[0];
    // Inside stroke of weight 4 on 100 × 50: 5000 − 92 × 42.
    assert!(near(f.area(v), 1_136.0, 1e-3), "{}", f.area(v));
    let siblings = f.doc.children(f.page);
    let at = siblings.iter().position(|s| *s == r).unwrap();
    assert_eq!(siblings[at + 1], v, "the outline sits just above the layer");
    assert!(
        f.doc
            .node(r)
            .unwrap()
            .data
            .get("strokePaints")
            .unwrap()
            .items()
            .is_empty()
    );
    let fill = f
        .doc
        .node(v)
        .unwrap()
        .data
        .get("fillPaints")
        .unwrap()
        .items()[0]
        .path(&["color", "r"])
        .and_then(Val::as_f32);
    assert_eq!(fill, Some(1.0));
    assert!(
        f.run(vec![EditOp::OutlineStroke { target: r }]).is_err(),
        "no stroke left"
    );
}
