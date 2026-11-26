//! Masks, layout grids, export settings and prototype ops.

use std::path::PathBuf;

use figstudio_fig::edit::{EditOp, History, execute_edit_ops};
use figstudio_fig::{Document, Guid, Val};

struct Fx {
    doc: Document,
    h: History,
    page: Guid,
}

impl Fx {
    fn new() -> Fx {
        let doc = Document::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
        )
        .unwrap();
        let page = doc.pages[0];
        Fx {
            doc,
            h: History::new(20),
            page,
        }
    }

    fn json(&mut self, op: serde_json::Value) -> Result<Vec<Guid>, String> {
        let op: EditOp = serde_json::from_value(op).map_err(|e| e.to_string())?;
        execute_edit_ops(&mut self.doc, &mut self.h, &[op], None, None, false)
            .map(|r| r.0.created_ids)
            .map_err(|e| e.to_string())
    }

    fn frame(&mut self, x: f64) -> Guid {
        let page = self.page.to_string();
        self.json(
            serde_json::json!({ "op": "create_shape", "kind": "FRAME", "parent": page,
            "x": x, "y": 0, "width": 200, "height": 100 }),
        )
        .unwrap()[0]
    }

    fn data(&self, id: Guid) -> &Val {
        &self.doc.node(id).unwrap().data
    }
}

#[test]
fn masks_grids_and_exports_are_written_and_checked() {
    let mut f = Fx::new();
    let fr = f.frame(0.0);
    let id = fr.to_string();
    f.json(serde_json::json!({ "op": "set_mask", "target": id, "mask": "outline" }))
        .unwrap();
    assert_eq!(f.data(fr).flag("mask"), Some(true));
    assert_eq!(f.data(fr).text("maskType"), Some("OUTLINE"));
    f.json(serde_json::json!({ "op": "set_mask", "target": id, "mask": null }))
        .unwrap();
    assert_eq!(f.data(fr).flag("mask"), Some(false));
    assert!(f.data(fr).get("maskType").is_none());

    f.json(
        serde_json::json!({ "op": "set_layout_grids", "target": id, "grids": [
            { "pattern": "columns", "count": 12, "gutter": 16, "offset": 24 },
            { "pattern": "rows", "count": 4, "align": "center", "size": 20, "hex": "#0000FF33" },
            { "pattern": "grid", "size": 8 }
        ]}),
    )
    .unwrap();
    let grids = f.data(fr).get("layoutGrids").unwrap().items();
    assert_eq!(grids.len(), 3);
    assert_eq!(grids[0].text("axis"), Some("X"));
    assert_eq!(grids[0].num("numSections"), Some(12.0));
    assert_eq!(grids[1].text("type"), Some("CENTER"));
    assert_eq!(grids[2].text("pattern"), Some("GRID"));

    f.json(serde_json::json!({ "op": "set_export_settings", "target": id, "settings": [
        { "format": "png", "scale": 2, "suffix": "@2x" }, { "format": "svg" }, { "format": "jpg", "width": 640 }
    ]}))
    .unwrap();
    let ex = f.data(fr).get("exportSettings").unwrap().items();
    assert_eq!(
        ex[0].path(&["constraint", "value"]).and_then(Val::as_f32),
        Some(2.0)
    );
    assert_eq!(
        ex[2].path(&["constraint", "type"]).and_then(Val::as_str),
        Some("CONTENT_WIDTH")
    );
    let reopened = Document::from_bytes(&f.doc.to_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened
            .node(fr)
            .unwrap()
            .data
            .get("layoutGrids")
            .unwrap()
            .items()
            .len(),
        3
    );

    let r = f
        .json(
            serde_json::json!({ "op": "create_shape", "kind": "RECTANGLE",
        "parent": f.page.to_string(), "x": 0, "y": 0, "width": 10, "height": 10 }),
        )
        .unwrap()[0]
        .to_string();
    for bad in [
        serde_json::json!({ "op": "set_layout_grids", "target": r, "grids": [] }),
        serde_json::json!({ "op": "set_layout_grids", "target": id, "grids": [{ "pattern": "rows", "align": "diagonal" }] }),
        serde_json::json!({ "op": "set_export_settings", "target": id, "settings": [{ "format": "bmp" }] }),
        serde_json::json!({ "op": "set_export_settings", "target": id, "settings": [{ "format": "png", "scale": 2, "width": 10 }] }),
        serde_json::json!({ "op": "set_mask", "target": id, "mask": "sparkly" }),
    ] {
        assert!(f.json(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn reactions_flow_starts_and_scrolling_are_written_and_checked() {
    let mut f = Fx::new();
    let (a, b) = (f.frame(0.0), f.frame(300.0));
    let (sa, sb) = (a.to_string(), b.to_string());
    f.json(serde_json::json!({ "op": "set_reactions", "target": sa, "reactions": [
        { "trigger": { "type": "on_click" }, "action": { "type": "navigate", "to": sb },
          "transition": "smart_animate", "duration": 0.3, "easing": "ease_out" },
        { "trigger": { "type": "after_timeout", "ms": 1500 }, "action": { "type": "url", "url": "https://example.com" } }
    ]}))
    .unwrap();
    let rs = f.data(a).get("prototypeInteractions").unwrap().items();
    assert_eq!(rs.len(), 2);
    let act = &rs[0].get("actions").unwrap().items()[0];
    assert_eq!(act.get("transitionNodeID").and_then(Val::as_guid), Some(b));
    assert_eq!(act.text("transitionType"), Some("SMART_ANIMATE"));
    assert_eq!(act.text("easingType"), Some("OUT_CUBIC"));
    assert_eq!(
        rs[1]
            .path(&["event", "transitionTimeout"])
            .and_then(Val::as_f32),
        Some(1.5)
    );
    assert_ne!(rs[0].get("id"), rs[1].get("id"));

    f.json(serde_json::json!({ "op": "set_flow_start", "frame": sa, "name": "Onboarding" }))
        .unwrap();
    assert!(f.data(a).get("prototypeStartingPoint").is_some());
    f.json(
        serde_json::json!({ "op": "set_scroll_behavior", "target": sa, "overflow": "vertical" }),
    )
    .unwrap();
    assert_eq!(f.data(a).text("scrollDirection"), Some("VERTICAL"));
    let reopened = Document::from_bytes(&f.doc.to_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened
            .node(a)
            .unwrap()
            .data
            .get("prototypeInteractions")
            .unwrap()
            .items()
            .len(),
        2
    );

    let missing = "9:9999";
    for bad in [
        serde_json::json!({ "op": "set_reactions", "target": sa, "reactions": [
            { "trigger": { "type": "on_click" }, "action": { "type": "navigate", "to": missing } }] }),
        serde_json::json!({ "op": "set_reactions", "target": sa, "reactions": [
            { "trigger": { "type": "on_click" }, "action": { "type": "back" }, "transition": "teleport" }] }),
        serde_json::json!({ "op": "set_scroll_behavior", "target": sa, "behavior": "floaty" }),
    ] {
        assert!(f.json(bad.clone()).is_err(), "{bad}");
    }
    // A flow cannot start inside a frame.
    let inner = f
        .json(
            serde_json::json!({ "op": "create_shape", "kind": "FRAME", "parent": sa,
        "x": 0, "y": 0, "width": 10, "height": 10 }),
        )
        .unwrap()[0]
        .to_string();
    assert!(
        f.json(serde_json::json!({ "op": "set_flow_start", "frame": inner, "name": "x" }))
            .is_err()
    );
}

#[test]
fn stored_reactions_read_back_as_the_specs_that_wrote_them() {
    use figstudio_fig::edit::{Action, ReactionSpec, Trigger, reactions_of};
    let mut f = Fx::new();
    let (a, b) = (f.frame(0.0), f.frame(300.0));
    let specs = vec![
        ReactionSpec {
            trigger: Trigger::OnClick,
            action: Action::Navigate { to: b },
            transition: Some("smart_animate".into()),
            duration: Some(0.3),
            easing: Some("ease_out".into()),
        },
        ReactionSpec {
            trigger: Trigger::AfterTimeout { ms: 1500.0 },
            action: Action::Back,
            transition: Some("instant".into()),
            duration: None,
            easing: None,
        },
    ];
    execute_edit_ops(
        &mut f.doc,
        &mut f.h,
        &[EditOp::SetReactions {
            target: a,
            reactions: specs.clone(),
        }],
        None,
        None,
        false,
    )
    .unwrap();
    let read = reactions_of(&f.doc, a).expect("readable");
    assert_eq!(read.len(), 2);
    assert_eq!(read[0].action, specs[0].action);
    assert_eq!(read[0].transition.as_deref(), Some("smart_animate"));
    assert!((read[0].duration.unwrap() - 0.3).abs() < 1e-6);
    assert_eq!(read[0].easing.as_deref(), Some("ease_out"));
    assert!(matches!(read[1].trigger, Trigger::AfterTimeout { ms } if (ms - 1500.0).abs() < 1e-3));
    assert_eq!(read[1].action, Action::Back);
    // A reaction with two actions cannot be said as a spec: nothing is offered for rewriting.
    let mut two = f.data(a).get("prototypeInteractions").unwrap().items()[0]
        .clone()
        .into_owned();
    let act = two.get("actions").unwrap().items()[0].clone().into_owned();
    two.set("actions", Val::Arr(vec![act.clone(), act]));
    let mut txn = figstudio_fig::edit::Txn::new("two actions");
    txn.set_field(a, "prototypeInteractions", Val::Arr(vec![two]));
    f.h.execute(&mut f.doc, txn).unwrap();
    assert!(reactions_of(&f.doc, a).is_none());
}
