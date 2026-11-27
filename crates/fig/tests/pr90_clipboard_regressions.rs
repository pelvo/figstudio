//! Regression tests for clipboard master reuse identity, parent-scoped subtree matching,
//! conservative handling of ambiguous siblings, and domain-separated node-reference vs
//! override-path remapping (Finding 4).

use std::path::PathBuf;

use figstudio_fig::clipboard::{encode_clipboard_html, encode_subset_kiwi, paste_kiwi_into};
use figstudio_fig::edit::{EditOp, History, Txn, execute_edit_ops, execute_paste_html};
use figstudio_fig::resolve::{InstancePath, resolve_instance, resolve_path};
use figstudio_fig::{Document, Guid, Node, Obj, Val, intern};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn open(name: &str) -> (Document, History) {
    (Document::open(fixture(name)).unwrap(), History::new(50))
}

fn run(doc: &mut Document, history: &mut History, ops: &[EditOp]) -> Vec<Guid> {
    execute_edit_ops(doc, history, ops, None, None, false)
        .unwrap()
        .0
        .created_ids
}

fn shape(kind: &str, parent: Guid, name: &str, x: f64, y: f64, w: f64, h: f64) -> EditOp {
    EditOp::CreateShape {
        id: None,
        kind: kind.into(),
        parent,
        name: Some(name.into()),
        x,
        y,
        width: w,
        height: h,
        corner_radius: 0.0,
        fill_hex: None,
        count: None,
        star_inner_scale: None,
    }
}

fn text_layer(parent: Guid, x: f64, y: f64, characters: &str) -> EditOp {
    EditOp::CreateText {
        id: None,
        parent,
        x,
        y,
        characters: characters.into(),
        font_family: "Inter".into(),
        font_style: "Regular".into(),
        font_size: 12.0,
        fill_hex: "#111111".into(),
    }
}

fn symbol_of(doc: &Document, instance: Guid) -> Guid {
    doc.node(instance)
        .unwrap()
        .data
        .path(&["symbolData", "symbolID"])
        .and_then(Val::as_guid)
        .unwrap()
}

fn set_chars(doc: &mut Document, history: &mut History, instance: Guid, path: &[Guid], s: &str) {
    run(
        doc,
        history,
        &[EditOp::SetInstanceOverride {
            instance,
            override_path: path.to_vec(),
            characters: Some(s.into()),
            visible: None,
            fill_hex: None,
            opacity: None,
        }],
    );
}

fn swap_child_order(doc: &mut Document, parent: Guid, a: Guid, b: Guid) {
    let pa = doc.node(a).unwrap().position.clone();
    let pb = doc.node(b).unwrap().position.clone();
    let pindex = |pos: String| {
        Val::Obj(Obj::from([
            ("guid", parent.to_val()),
            ("position", Val::Str(pos)),
        ]))
    };
    let mut swap = Txn::new("swap sibling order");
    swap.set_field(a, "parentIndex", pindex(pb));
    swap.set_field(b, "parentIndex", pindex(pa));
    doc.apply_txn(&swap).unwrap();
}

fn shift_guids(doc: &mut Document, history: &mut History, page: Guid) {
    let ops: Vec<EditOp> = (0..6)
        .map(|i| {
            shape(
                "RECTANGLE",
                page,
                "Shift",
                f64::from(i) * 10.0,
                0.0,
                8.0,
                8.0,
            )
        })
        .collect();
    run(doc, history, &ops);
}

fn node_val(
    id: Guid,
    kind: &str,
    name: &str,
    parent: Option<(Guid, &str)>,
    extra: &[(&str, Val)],
) -> Val {
    let mut obj = Obj::new();
    obj.insert("guid", id.to_val());
    obj.insert("type", Val::Enum(intern(kind)));
    obj.insert("name", Val::Str(name.into()));
    obj.insert("phase", Val::Enum(intern("CREATED")));
    obj.insert("visible", Val::Bool(true));
    if let Some((p, pos)) = parent {
        obj.insert(
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", p.to_val()),
                ("position", Val::Str(pos.into())),
            ])),
        );
    }
    for (k, v) in extra {
        obj.insert(k, v.clone());
    }
    Val::Obj(obj)
}

fn text_data(s: &str) -> Val {
    Val::Obj(Obj::from([("characters", Val::Str(s.into()))]))
}

fn guid_path(guids: &[Guid]) -> Val {
    Val::Obj(Obj::from([(
        "guids",
        Val::Arr(guids.iter().map(|g| g.to_val()).collect()),
    )]))
}

fn override_paths(doc: &Document, instance: Guid, field: &[&str]) -> Vec<Vec<Guid>> {
    doc.node(instance)
        .unwrap()
        .data
        .path(field)
        .map(Val::items)
        .unwrap_or_default()
        .iter()
        .map(|e| {
            e.path(&["guidPath", "guids"])
                .map(Val::items)
                .unwrap_or_default()
                .iter()
                .filter_map(Val::as_guid)
                .collect()
        })
        .collect()
}

// ---- 4.1: same-document duplicate identity survives Kiwi serialization ------------------

#[test]
fn duplicate_node_reuses_keyed_master_with_rotated_fractional_and_reordered_fields() {
    let (mut doc, mut history) = open("structure.fig");
    let page = doc.pages[0];
    let master = run(
        &mut doc,
        &mut history,
        &[shape("FRAME", page, "Card", 0.0, 0.0, 160.0, 90.0)],
    )[0];
    let rotated = run(
        &mut doc,
        &mut history,
        &[shape(
            "RECTANGLE",
            master,
            "Diamond",
            20.0,
            20.0,
            40.0,
            40.0,
        )],
    )[0];
    let label = run(
        &mut doc,
        &mut history,
        &[text_layer(master, 4.0, 60.0, "Title")],
    )[0];

    // Rotate 45 degrees (f64 cos/sin in memory vs f32 after Kiwi round-trip) and set a
    // non-dyadic fractional size with reversed Obj field insertion order.
    run(
        &mut doc,
        &mut history,
        &[
            EditOp::Rotate {
                target: rotated,
                degrees: 45.0,
            },
            EditOp::CreateComponent { frame: master },
        ],
    );
    let mut setup = Txn::new("key and fractional size");
    setup.set_field(master, "key", Val::Str("rotated-dup-key".into()));
    setup.set_field(
        label,
        "size",
        Val::Obj(Obj::from([
            ("y", Val::Float(50.0 / 7.0)),
            ("x", Val::Float(100.0 / 3.0)),
        ])),
    );
    doc.apply_txn(&setup).unwrap();

    let inst = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 200.0,
            y: 0.0,
        }],
    )[0];
    let label_key = doc.node(label).unwrap().override_key();
    set_chars(&mut doc, &mut history, inst, &[label_key], "Custom");

    let dup = run(
        &mut doc,
        &mut history,
        &[EditOp::DuplicateNode {
            target: inst,
            dx: 20.0,
            dy: 0.0,
        }],
    )[0];
    assert_eq!(
        symbol_of(&doc, dup),
        master,
        "DuplicateNode imported a second master instead of reusing its own"
    );

    // Same-document clipboard HTML copy + paste must also reuse the master.
    let html = encode_clipboard_html(&doc, &[inst], 1, None, None).unwrap();
    let (pasted, _) = execute_paste_html(&mut doc, &mut history, &html, page, (40.0, 0.0)).unwrap();
    assert_eq!(
        symbol_of(&doc, pasted[0]),
        master,
        "same-document paste imported a second master instead of reusing its own"
    );

    let keyed_count = doc
        .nodes
        .iter()
        .filter(|n| n.kind == "SYMBOL" && n.data.text("key") == Some("rotated-dup-key"))
        .count();
    assert_eq!(keyed_count, 1, "expected a single shared master SYMBOL");

    for copy_id in [dup, pasted[0]] {
        let resolved =
            resolve_path(&doc, &InstancePath::new(vec![copy_id, label_key]).unwrap()).unwrap();
        assert_eq!(
            resolved
                .data
                .path(&["textData", "characters"])
                .and_then(Val::as_str),
            Some("Custom")
        );
    }
}

// ---- 4.2: ambiguous siblings never misroute ---------------------------------------------

#[test]
fn ambiguous_unkeyed_siblings_never_misroute_overrides_after_z_order_swap() {
    let (mut src, mut src_h) = open("structure.fig");
    let src_page = src.pages[0];
    let master = run(
        &mut src,
        &mut src_h,
        &[shape("FRAME", src_page, "List", 0.0, 0.0, 120.0, 80.0)],
    )[0];
    let first = run(
        &mut src,
        &mut src_h,
        &[text_layer(master, 4.0, 4.0, "Alpha")],
    )[0];
    let second = run(
        &mut src,
        &mut src_h,
        &[text_layer(master, 4.0, 4.0, "Beta")],
    )[0];
    // Give both unkeyed siblings identical (kind, name, transform, size) but distinct text.
    run(
        &mut src,
        &mut src_h,
        &[
            EditOp::Rename {
                target: first,
                name: "Item".into(),
            },
            EditOp::Rename {
                target: second,
                name: "Item".into(),
            },
            EditOp::Resize {
                target: first,
                width: 40.0,
                height: 20.0,
            },
            EditOp::Resize {
                target: second,
                width: 40.0,
                height: 20.0,
            },
            EditOp::CreateComponent { frame: master },
        ],
    );
    let mut key = Txn::new("key");
    key.set_field(master, "key", Val::Str("ambiguous-siblings-key".into()));
    src.apply_txn(&key).unwrap();

    let inst = run(
        &mut src,
        &mut src_h,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: src_page,
            x: 200.0,
            y: 0.0,
        }],
    )[0];
    set_chars(&mut src, &mut src_h, inst, &[first], "Overridden");
    let html = encode_clipboard_html(&src, &[inst], 1, None, None).unwrap();

    let (mut dst, mut dst_h) = open("corners.fig");
    let dst_page = dst.pages[0];
    shift_guids(&mut dst, &mut dst_h, dst_page);

    let (p1, _) = execute_paste_html(&mut dst, &mut dst_h, &html, dst_page, (0.0, 0.0)).unwrap();
    let dst_master = symbol_of(&dst, p1[0]);
    let kids = dst.children(dst_master).to_vec();
    assert_eq!(kids.len(), 2);

    // Swap the two ambiguous siblings' z-order in dst.
    swap_child_order(&mut dst, dst_master, kids[0], kids[1]);

    let (p2, _) = execute_paste_html(&mut dst, &mut dst_h, &html, dst_page, (150.0, 0.0)).unwrap();

    let mut texts: Vec<String> = resolve_instance(&dst, p2[0])
        .unwrap()
        .nodes
        .iter()
        .filter_map(|r| {
            r.data
                .path(&["textData", "characters"])
                .and_then(Val::as_str)
                .map(str::to_owned)
        })
        .collect();
    texts.sort();
    assert_eq!(
        texts,
        vec!["Beta".to_string(), "Overridden".to_string()],
        "override misrouted onto the Beta sibling instead of Alpha"
    );
    assert_ne!(
        symbol_of(&dst, p2[0]),
        dst_master,
        "ambiguous unkeyed siblings should not be paired by guessing"
    );
}

#[test]
fn unkeyed_same_name_siblings_do_not_misroute_when_positions_swap() {
    // Even when two unkeyed same-name siblings sit at different positions, swapping their
    // positions in dst must not match them by guessed floats and misroute the override.
    let (mut src, mut src_h) = open("structure.fig");
    let src_page = src.pages[0];
    let master = run(
        &mut src,
        &mut src_h,
        &[shape("FRAME", src_page, "Stack", 0.0, 0.0, 120.0, 80.0)],
    )[0];
    let top = run(&mut src, &mut src_h, &[text_layer(master, 4.0, 4.0, "Top")])[0];
    let bottom = run(
        &mut src,
        &mut src_h,
        &[text_layer(master, 4.0, 40.0, "Bottom")],
    )[0];
    run(
        &mut src,
        &mut src_h,
        &[
            EditOp::Rename {
                target: top,
                name: "Item".into(),
            },
            EditOp::Rename {
                target: bottom,
                name: "Item".into(),
            },
            EditOp::Resize {
                target: top,
                width: 40.0,
                height: 20.0,
            },
            EditOp::Resize {
                target: bottom,
                width: 40.0,
                height: 20.0,
            },
            EditOp::CreateComponent { frame: master },
        ],
    );
    let mut key = Txn::new("key");
    key.set_field(master, "key", Val::Str("swapped-float-siblings-key".into()));
    src.apply_txn(&key).unwrap();

    let inst = run(
        &mut src,
        &mut src_h,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: src_page,
            x: 200.0,
            y: 0.0,
        }],
    )[0];
    set_chars(&mut src, &mut src_h, inst, &[top], "OverriddenTop");
    let html = encode_clipboard_html(&src, &[inst], 1, None, None).unwrap();

    let (mut dst, mut dst_h) = open("corners.fig");
    let dst_page = dst.pages[0];
    shift_guids(&mut dst, &mut dst_h, dst_page);

    let (p1, _) = execute_paste_html(&mut dst, &mut dst_h, &html, dst_page, (0.0, 0.0)).unwrap();
    let dst_master = symbol_of(&dst, p1[0]);
    let kids = dst.children(dst_master).to_vec();
    // Swap the two siblings' transforms in dst (Top moves to y=40, Bottom moves to y=4).
    run(
        &mut dst,
        &mut dst_h,
        &[
            EditOp::Move {
                target: kids[0],
                x: 4.0,
                y: 40.0,
                relative: false,
            },
            EditOp::Move {
                target: kids[1],
                x: 4.0,
                y: 4.0,
                relative: false,
            },
        ],
    );

    let (p2, _) = execute_paste_html(&mut dst, &mut dst_h, &html, dst_page, (150.0, 0.0)).unwrap();
    let mut texts: Vec<String> = resolve_instance(&dst, p2[0])
        .unwrap()
        .nodes
        .iter()
        .filter_map(|r| {
            r.data
                .path(&["textData", "characters"])
                .and_then(Val::as_str)
                .map(str::to_owned)
        })
        .collect();
    texts.sort();
    assert_eq!(
        texts,
        vec!["Bottom".to_string(), "OverriddenTop".to_string()],
        "float matching misrouted Top's override onto Bottom"
    );
}

// ---- 4.3: subtree matching occurs within parents ----------------------------------------

#[test]
fn reordered_sibling_containers_reuse_master_and_match_children_within_parents() {
    let (mut src, mut src_h) = open("structure.fig");
    let src_page = src.pages[0];
    let master = run(
        &mut src,
        &mut src_h,
        &[shape("FRAME", src_page, "Table", 0.0, 0.0, 160.0, 100.0)],
    )[0];
    let row1 = run(
        &mut src,
        &mut src_h,
        &[shape("FRAME", master, "Row 1", 0.0, 0.0, 160.0, 40.0)],
    )[0];
    let row2 = run(
        &mut src,
        &mut src_h,
        &[shape("FRAME", master, "Row 2", 0.0, 50.0, 160.0, 40.0)],
    )[0];
    let r1_label = run(
        &mut src,
        &mut src_h,
        &[text_layer(row1, 4.0, 4.0, "Row1Base")],
    )[0];
    let r2_label = run(
        &mut src,
        &mut src_h,
        &[text_layer(row2, 4.0, 4.0, "Row2Base")],
    )[0];
    run(
        &mut src,
        &mut src_h,
        &[
            EditOp::Rename {
                target: r1_label,
                name: "Label".into(),
            },
            EditOp::Rename {
                target: r2_label,
                name: "Label".into(),
            },
            EditOp::Resize {
                target: r1_label,
                width: 40.0,
                height: 16.0,
            },
            EditOp::Resize {
                target: r2_label,
                width: 40.0,
                height: 16.0,
            },
            EditOp::CreateComponent { frame: master },
        ],
    );
    let mut key = Txn::new("key");
    key.set_field(master, "key", Val::Str("reordered-rows-key".into()));
    src.apply_txn(&key).unwrap();

    let inst = run(
        &mut src,
        &mut src_h,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: src_page,
            x: 200.0,
            y: 0.0,
        }],
    )[0];
    set_chars(&mut src, &mut src_h, inst, &[r1_label], "UpdatedRow1");
    let html = encode_clipboard_html(&src, &[inst], 1, None, None).unwrap();

    let (mut dst, mut dst_h) = open("corners.fig");
    let dst_page = dst.pages[0];
    shift_guids(&mut dst, &mut dst_h, dst_page);

    let (p1, _) = execute_paste_html(&mut dst, &mut dst_h, &html, dst_page, (0.0, 0.0)).unwrap();
    let dst_master = symbol_of(&dst, p1[0]);
    let rows = dst.children(dst_master).to_vec();
    assert_eq!(rows.len(), 2);

    // Swap Row 1 and Row 2 order in dst's master.
    swap_child_order(&mut dst, dst_master, rows[0], rows[1]);

    let (p2, _) = execute_paste_html(&mut dst, &mut dst_h, &html, dst_page, (200.0, 0.0)).unwrap();
    assert_eq!(
        symbol_of(&dst, p2[0]),
        dst_master,
        "reordered sibling containers with identical child signatures failed to reuse master"
    );

    let dst_row1 = dst
        .children(dst_master)
        .iter()
        .copied()
        .find(|&id| dst.node(id).unwrap().name == "Row 1")
        .unwrap();
    let dst_row2 = dst
        .children(dst_master)
        .iter()
        .copied()
        .find(|&id| dst.node(id).unwrap().name == "Row 2")
        .unwrap();
    let dst_r1_label = dst.children(dst_row1)[0];
    let dst_r2_label = dst.children(dst_row2)[0];

    let r1_resolved =
        resolve_path(&dst, &InstancePath::new(vec![p2[0], dst_r1_label]).unwrap()).unwrap();
    let r2_resolved =
        resolve_path(&dst, &InstancePath::new(vec![p2[0], dst_r2_label]).unwrap()).unwrap();
    assert_eq!(
        r1_resolved
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str),
        Some("UpdatedRow1"),
        "Row 1/Label lost its override"
    );
    assert_eq!(
        r2_resolved
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str),
        Some("Row2Base"),
        "Row 2/Label received Row 1's override"
    );
}

#[test]
fn reordered_state_group_variants_reuse_component_set_even_with_shared_variant_override_keys() {
    let mut src = Document::open(fixture("structure.fig")).unwrap();
    let doc_id = Guid(0, 0);
    let page_id = Guid(0, 1);
    let sg_id = Guid(1, 1);
    let v_default = Guid(1, 2);
    let v_hover = Guid(1, 3);
    let l_default = Guid(1, 4);
    let l_hover = Guid(1, 5);
    let inst_id = Guid(1, 6);
    let shared_label_key = Guid(9, 10);

    let nodes: Vec<Node> = [
        node_val(doc_id, "DOCUMENT", "Document", None, &[]),
        node_val(page_id, "CANVAS", "Page 1", Some((doc_id, "!")), &[]),
        node_val(
            sg_id,
            "FRAME",
            "Button",
            Some((page_id, "!")),
            &[
                ("isStateGroup", Val::Bool(true)),
                ("key", Val::Str("sg-variants-key".into())),
            ],
        ),
        node_val(
            v_default,
            "SYMBOL",
            "State=Default",
            Some((sg_id, "!")),
            &[],
        ),
        node_val(v_hover, "SYMBOL", "State=Hover", Some((sg_id, "\"")), &[]),
        node_val(
            l_default,
            "TEXT",
            "Label",
            Some((v_default, "!")),
            &[
                ("overrideKey", shared_label_key.to_val()),
                ("textData", text_data("Default")),
            ],
        ),
        node_val(
            l_hover,
            "TEXT",
            "Label",
            Some((v_hover, "!")),
            &[
                ("overrideKey", shared_label_key.to_val()),
                ("textData", text_data("Hover")),
            ],
        ),
        node_val(
            inst_id,
            "INSTANCE",
            "Use",
            Some((page_id, "\"")),
            &[(
                "symbolData",
                Val::Obj(Obj::from([
                    ("symbolID", v_default.to_val()),
                    (
                        "symbolOverrides",
                        Val::Arr(vec![Val::Obj(Obj::from([
                            ("guidPath", guid_path(&[shared_label_key])),
                            ("textData", text_data("Pressed")),
                        ]))]),
                    ),
                ])),
            )],
        ),
    ]
    .into_iter()
    .map(|v| Node::from_data(v).unwrap())
    .collect();
    src.set_nodes(nodes).unwrap();

    let html = encode_clipboard_html(&src, &[inst_id], 1, None, None).unwrap();

    let (mut dst, mut dst_h) = open("corners.fig");
    let dst_page = dst.pages[0];
    shift_guids(&mut dst, &mut dst_h, dst_page);

    let (p1, _) = execute_paste_html(&mut dst, &mut dst_h, &html, dst_page, (0.0, 0.0)).unwrap();
    let dst_v_default = symbol_of(&dst, p1[0]);
    let dst_sg = dst.node(dst_v_default).unwrap().parent.unwrap();
    let variants = dst.children(dst_sg).to_vec();
    assert_eq!(variants.len(), 2);

    // Swap variant order inside the pasted STATE_GROUP.
    swap_child_order(&mut dst, dst_sg, variants[0], variants[1]);

    let (p2, _) = execute_paste_html(&mut dst, &mut dst_h, &html, dst_page, (150.0, 0.0)).unwrap();
    assert_eq!(
        symbol_of(&dst, p2[0]),
        dst_v_default,
        "reordered STATE_GROUP variants sharing a per-variant overrideKey failed to reuse"
    );
    let resolved = resolve_path(
        &dst,
        &InstancePath::new(vec![p2[0], shared_label_key]).unwrap(),
    )
    .unwrap();
    assert_eq!(
        resolved
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str),
        Some("Pressed")
    );
}

// ---- 4.4: node references and override path keys remap in their own domains -------------

#[test]
fn self_keyed_override_key_preserved_on_reused_master_while_node_references_remap() {
    let mut src = Document::open(fixture("structure.fig")).unwrap();
    let doc_id = Guid(0, 0);
    let page_id = Guid(0, 1);
    let src_master = Guid(1, 1);
    let src_label = Guid(1, 10); // self-keyed: overrideKey == id == 1:10
    let src_inner_inst = Guid(1, 11);
    let src_icon_a = Guid(1, 20);
    let src_icon_b = Guid(1, 21);
    let src_style = Guid(1, 30);
    let src_inst = Guid(1, 40);

    // Instance overrides BOTH:
    // - self-keyed `1:10` (textData + fill style refs to `src_style`) in symbolOverrides & derivedSymbolData
    // - inner instance `1:11` swapped via `overriddenSymbolID` node ref to `src_icon_b`
    // - nested 2-segment override path `[1:11, 1:22]` into `src_icon_b`'s self-keyed leaf `1:22`
    let src_icon_b_leaf = Guid(1, 22);
    let sym_data = Val::Obj(Obj::from([
        ("symbolID", src_master.to_val()),
        (
            "symbolOverrides",
            Val::Arr(vec![
                Val::Obj(Obj::from([
                    ("guidPath", guid_path(&[src_label])),
                    ("textData", text_data("OverriddenLabel")),
                    (
                        "styleIdForFill",
                        Val::Obj(Obj::from([("guid", src_style.to_val())])),
                    ),
                    ("inheritFillStyleID", src_style.to_val()),
                ])),
                Val::Obj(Obj::from([
                    ("guidPath", guid_path(&[src_inner_inst])),
                    ("overriddenSymbolID", src_icon_b.to_val()),
                ])),
                Val::Obj(Obj::from([
                    ("guidPath", guid_path(&[src_inner_inst, src_icon_b_leaf])),
                    ("textData", text_data("NestedLeafOverride")),
                ])),
            ]),
        ),
    ]));
    let derived = Val::Arr(vec![Val::Obj(Obj::from([
        ("guidPath", guid_path(&[src_label])),
        (
            "size",
            Val::Obj(Obj::from([
                ("x", Val::Float(88.0)),
                ("y", Val::Float(20.0)),
            ])),
        ),
    ]))]);

    // Also include a connector-style arbitrary node reference pointing directly to `src_label`
    // (1:10) so the exact same GUID 1:10 must remap to `dst_label` (5:20) in node-reference
    // position while staying `1:10` inside `guidPath.guids`!
    let src_nodes: Vec<Node> = [
        node_val(doc_id, "DOCUMENT", "Document", None, &[]),
        node_val(page_id, "CANVAS", "Page 1", Some((doc_id, "!")), &[]),
        node_val(
            src_style,
            "RECTANGLE",
            "Accent",
            Some((page_id, "!")),
            &[
                ("styleType", Val::Enum(intern("FILL"))),
                ("key", Val::Str("domain-style-key".into())),
            ],
        ),
        node_val(
            src_icon_a,
            "SYMBOL",
            "IconA",
            Some((page_id, "\"")),
            &[("key", Val::Str("domain-icon-a".into()))],
        ),
        node_val(
            src_icon_b,
            "SYMBOL",
            "IconB",
            Some((page_id, "#")),
            &[("key", Val::Str("domain-icon-b".into()))],
        ),
        node_val(
            src_icon_b_leaf,
            "TEXT",
            "Glyph",
            Some((src_icon_b, "!")),
            &[
                ("overrideKey", src_icon_b_leaf.to_val()),
                ("textData", text_data("B")),
            ],
        ),
        node_val(
            src_master,
            "SYMBOL",
            "Card",
            Some((page_id, "$")),
            &[("key", Val::Str("domain-card-key".into()))],
        ),
        node_val(
            src_label,
            "TEXT",
            "Label",
            Some((src_master, "!")),
            &[
                ("overrideKey", src_label.to_val()),
                ("textData", text_data("Base")),
            ],
        ),
        node_val(
            src_inner_inst,
            "INSTANCE",
            "IconSlot",
            Some((src_master, "\"")),
            &[
                ("overrideKey", src_inner_inst.to_val()),
                (
                    "symbolData",
                    Val::Obj(Obj::from([("symbolID", src_icon_a.to_val())])),
                ),
            ],
        ),
        node_val(
            src_inst,
            "INSTANCE",
            "CardUse",
            Some((page_id, "%")),
            &[
                ("symbolData", sym_data),
                ("derivedSymbolData", derived),
                (
                    "connectorStart",
                    Val::Obj(Obj::from([("endpointNodeID", src_label.to_val())])),
                ),
            ],
        ),
    ]
    .into_iter()
    .map(|v| Node::from_data(v).unwrap())
    .collect();
    src.set_nodes(src_nodes).unwrap();

    let kiwi = encode_subset_kiwi(&src, &[src_inst], 1, "domain-test").unwrap();

    // Destination has existing masters with DIFFERENT node IDs (5:*) keeping overrideKey == 1:10 / 1:11 / 1:22.
    let mut dst = Document::open(fixture("structure.fig")).unwrap();
    let dst_doc = Guid(0, 0);
    let dst_page = Guid(0, 1);
    let dst_master = Guid(5, 1);
    let dst_label = Guid(5, 20);
    let dst_inner_inst = Guid(5, 21);
    let dst_icon_a = Guid(5, 30);
    let dst_icon_b = Guid(5, 31);
    let dst_icon_b_leaf = Guid(5, 32);
    let dst_style = Guid(5, 40);

    let dst_nodes: Vec<Node> = [
        node_val(dst_doc, "DOCUMENT", "Document", None, &[]),
        node_val(dst_page, "CANVAS", "Page 1", Some((dst_doc, "!")), &[]),
        node_val(
            dst_style,
            "RECTANGLE",
            "Accent",
            Some((dst_page, "!")),
            &[
                ("styleType", Val::Enum(intern("FILL"))),
                ("key", Val::Str("domain-style-key".into())),
            ],
        ),
        node_val(
            dst_icon_a,
            "SYMBOL",
            "IconA",
            Some((dst_page, "\"")),
            &[("key", Val::Str("domain-icon-a".into()))],
        ),
        node_val(
            dst_icon_b,
            "SYMBOL",
            "IconB",
            Some((dst_page, "#")),
            &[("key", Val::Str("domain-icon-b".into()))],
        ),
        node_val(
            dst_icon_b_leaf,
            "TEXT",
            "Glyph",
            Some((dst_icon_b, "!")),
            &[
                ("overrideKey", src_icon_b_leaf.to_val()),
                ("textData", text_data("B")),
            ],
        ),
        node_val(
            dst_master,
            "SYMBOL",
            "Card",
            Some((dst_page, "$")),
            &[("key", Val::Str("domain-card-key".into()))],
        ),
        node_val(
            dst_label,
            "TEXT",
            "Label",
            Some((dst_master, "!")),
            &[
                ("overrideKey", src_label.to_val()),
                ("textData", text_data("Base")),
            ],
        ),
        node_val(
            dst_inner_inst,
            "INSTANCE",
            "IconSlot",
            Some((dst_master, "\"")),
            &[
                ("overrideKey", src_inner_inst.to_val()),
                (
                    "symbolData",
                    Val::Obj(Obj::from([("symbolID", dst_icon_a.to_val())])),
                ),
            ],
        ),
    ]
    .into_iter()
    .map(|v| Node::from_data(v).unwrap())
    .collect();
    dst.set_nodes(dst_nodes).unwrap();

    let (txn, pasted) = paste_kiwi_into(&dst, &kiwi, dst_page, (0.0, 0.0)).unwrap();
    let mut history = History::new(10);
    history.execute(&mut dst, txn).unwrap();
    let pasted_inst = pasted[0];

    // Node-reference domain: symbolID remapped to dst_master (5:1) and arbitrary GUID ref
    // `connectorStart.endpointNodeID` for 1:10 remapped to dst_label (5:20).
    assert_eq!(symbol_of(&dst, pasted_inst), dst_master);
    assert_eq!(
        dst.node(pasted_inst)
            .unwrap()
            .data
            .path(&["connectorStart", "endpointNodeID"])
            .and_then(Val::as_guid),
        Some(dst_label),
        "arbitrary node GUID reference to 1:10 should still remap to dst_label (5:20)"
    );

    // Override-path domain: guidPath.guids stayed [1:10], [1:11], and [1:11, 1:22], NOT 5:*.
    assert_eq!(
        override_paths(&dst, pasted_inst, &["symbolData", "symbolOverrides"]),
        vec![
            vec![src_label],
            vec![src_inner_inst],
            vec![src_inner_inst, src_icon_b_leaf],
        ],
        "symbolOverrides guidPath was rewritten with destination node IDs instead of overrideKeys"
    );
    assert_eq!(
        override_paths(&dst, pasted_inst, &["derivedSymbolData"]),
        vec![vec![src_label]],
        "derivedSymbolData guidPath was rewritten with destination node ID instead of overrideKey"
    );

    // Node references inside the SAME override entries still remapped to dst node IDs!
    let overrides = dst
        .node(pasted_inst)
        .unwrap()
        .data
        .path(&["symbolData", "symbolOverrides"])
        .unwrap()
        .items();
    assert_eq!(
        overrides[0]
            .path(&["styleIdForFill", "guid"])
            .and_then(Val::as_guid),
        Some(dst_style),
        "styleIdForFill node reference inside override entry was not remapped"
    );
    assert_eq!(
        overrides[0]
            .get("inheritFillStyleID")
            .and_then(Val::as_guid),
        Some(dst_style),
        "inheritFillStyleID node reference inside override entry was not remapped"
    );
    assert_eq!(
        overrides[1]
            .get("overriddenSymbolID")
            .and_then(Val::as_guid),
        Some(dst_icon_b),
        "overriddenSymbolID node reference inside override entry was not remapped"
    );

    // End-to-end resolution via resolve_instance sees both top-level and nested overrides.
    let walk = resolve_instance(&dst, pasted_inst).unwrap();
    let resolved_label = walk
        .nodes
        .iter()
        .find(|r| r.node == dst_label)
        .expect("resolved label");
    assert_eq!(
        resolved_label
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str),
        Some("OverriddenLabel"),
        "resolve_instance lost the override on self-keyed reused master"
    );
    let resolved_nested_leaf = walk
        .nodes
        .iter()
        .find(|r| r.node == dst_icon_b_leaf)
        .expect("resolved nested leaf");
    assert_eq!(
        resolved_nested_leaf
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str),
        Some("NestedLeafOverride"),
        "resolve_instance lost the 2-segment nested override on self-keyed reused masters"
    );
}

#[test]
fn reused_master_remaps_source_node_id_override_path_to_destination_override_key() {
    // Source descendant has id=1:10 and explicit overrideKey=9:42; destination descendant has
    // id=5:20 and overrideKey=9:42. If the source instance's guidPath targeted [1:10] (s.id),
    // remapping onto the reused destination master must target [9:42] (d.override_key()), not [5:20].
    let mut src = Document::open(fixture("structure.fig")).unwrap();
    let doc_id = Guid(0, 0);
    let page_id = Guid(0, 1);
    let src_master = Guid(1, 1);
    let src_label = Guid(1, 10);
    let explicit_key = Guid(9, 42);
    let src_inst = Guid(1, 20);

    let src_nodes: Vec<Node> = [
        node_val(doc_id, "DOCUMENT", "Document", None, &[]),
        node_val(page_id, "CANVAS", "Page 1", Some((doc_id, "!")), &[]),
        node_val(
            src_master,
            "SYMBOL",
            "Master",
            Some((page_id, "!")),
            &[("key", Val::Str("explicit-id-target-key".into()))],
        ),
        node_val(
            src_label,
            "TEXT",
            "Label",
            Some((src_master, "!")),
            &[
                ("overrideKey", explicit_key.to_val()),
                ("textData", text_data("Base")),
            ],
        ),
        node_val(
            src_inst,
            "INSTANCE",
            "Use",
            Some((page_id, "\"")),
            &[(
                "symbolData",
                Val::Obj(Obj::from([
                    ("symbolID", src_master.to_val()),
                    (
                        "symbolOverrides",
                        Val::Arr(vec![Val::Obj(Obj::from([
                            ("guidPath", guid_path(&[src_label])),
                            ("textData", text_data("BySrcId")),
                        ]))]),
                    ),
                ])),
            )],
        ),
    ]
    .into_iter()
    .map(|v| Node::from_data(v).unwrap())
    .collect();
    src.set_nodes(src_nodes).unwrap();

    let kiwi = encode_subset_kiwi(&src, &[src_inst], 1, "id-target").unwrap();

    let mut dst = Document::open(fixture("structure.fig")).unwrap();
    let dst_doc = Guid(0, 0);
    let dst_page = Guid(0, 1);
    let dst_master = Guid(5, 1);
    let dst_label = Guid(5, 20);

    let dst_nodes: Vec<Node> = [
        node_val(dst_doc, "DOCUMENT", "Document", None, &[]),
        node_val(dst_page, "CANVAS", "Page 1", Some((dst_doc, "!")), &[]),
        node_val(
            dst_master,
            "SYMBOL",
            "Master",
            Some((dst_page, "!")),
            &[("key", Val::Str("explicit-id-target-key".into()))],
        ),
        node_val(
            dst_label,
            "TEXT",
            "Label",
            Some((dst_master, "!")),
            &[
                ("overrideKey", explicit_key.to_val()),
                ("textData", text_data("Base")),
            ],
        ),
    ]
    .into_iter()
    .map(|v| Node::from_data(v).unwrap())
    .collect();
    dst.set_nodes(dst_nodes).unwrap();

    let (txn, pasted) = paste_kiwi_into(&dst, &kiwi, dst_page, (0.0, 0.0)).unwrap();
    let mut history = History::new(10);
    history.execute(&mut dst, txn).unwrap();
    let pasted_inst = pasted[0];

    assert_eq!(symbol_of(&dst, pasted_inst), dst_master);
    assert_eq!(
        override_paths(&dst, pasted_inst, &["symbolData", "symbolOverrides"]),
        vec![vec![explicit_key]],
        "guidPath should remap to destination override_key(), not destination node id"
    );
    let resolved = resolve_instance(&dst, pasted_inst).unwrap();
    assert_eq!(
        resolved.nodes[0]
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str),
        Some("BySrcId")
    );
}

#[test]
fn fresh_paste_of_self_keyed_master_preserves_override_key_for_subsequent_reuse_after_rename() {
    let mut src = Document::open(fixture("structure.fig")).unwrap();
    let doc_id = Guid(0, 0);
    let page_id = Guid(0, 1);
    let src_master = Guid(1, 1);
    let src_label = Guid(1, 10); // self-keyed: overrideKey == id == 1:10
    let src_inst = Guid(1, 20);

    let src_nodes: Vec<Node> = [
        node_val(doc_id, "DOCUMENT", "Document", None, &[]),
        node_val(page_id, "CANVAS", "Page 1", Some((doc_id, "!")), &[]),
        node_val(
            src_master,
            "SYMBOL",
            "Master",
            Some((page_id, "!")),
            &[("key", Val::Str("fresh-self-keyed".into()))],
        ),
        node_val(
            src_label,
            "TEXT",
            "Label",
            Some((src_master, "!")),
            &[
                ("overrideKey", src_label.to_val()),
                ("textData", text_data("Base")),
            ],
        ),
        node_val(
            src_inst,
            "INSTANCE",
            "Use",
            Some((page_id, "\"")),
            &[(
                "symbolData",
                Val::Obj(Obj::from([
                    ("symbolID", src_master.to_val()),
                    (
                        "symbolOverrides",
                        Val::Arr(vec![Val::Obj(Obj::from([
                            ("guidPath", guid_path(&[src_label])),
                            ("textData", text_data("FirstPasteOverride")),
                        ]))]),
                    ),
                ])),
            )],
        ),
    ]
    .into_iter()
    .map(|v| Node::from_data(v).unwrap())
    .collect();
    src.set_nodes(src_nodes).unwrap();

    let html = encode_clipboard_html(&src, &[src_inst], 1, None, None).unwrap();

    let (mut dst, mut dst_h) = open("corners.fig");
    let dst_page = dst.pages[0];
    shift_guids(&mut dst, &mut dst_h, dst_page);

    // First paste imports the master fresh with new node IDs, keeping overrideKey = 1:10.
    let (p1, _) = execute_paste_html(&mut dst, &mut dst_h, &html, dst_page, (0.0, 0.0)).unwrap();
    let dst_master = symbol_of(&dst, p1[0]);
    let dst_label = dst.children(dst_master)[0];
    assert_ne!(dst_label, src_label);
    assert_eq!(dst.node(dst_label).unwrap().override_key(), src_label);
    assert_eq!(
        resolve_instance(&dst, p1[0]).unwrap().nodes[0]
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str),
        Some("FirstPasteOverride")
    );

    // Rename the layer in dst; second paste must still reuse the master via overrideKey 1:10.
    run(
        &mut dst,
        &mut dst_h,
        &[EditOp::Rename {
            target: dst_label,
            name: "RenamedLabel".into(),
        }],
    );

    let (p2, _) = execute_paste_html(&mut dst, &mut dst_h, &html, dst_page, (150.0, 0.0)).unwrap();
    assert_eq!(
        symbol_of(&dst, p2[0]),
        dst_master,
        "second paste should reuse master by preserved explicit overrideKey even after rename"
    );
    assert_eq!(
        resolve_instance(&dst, p2[0]).unwrap().nodes[0]
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str),
        Some("FirstPasteOverride")
    );
}

#[test]
fn duplicate_node_prefers_its_own_master_when_multiple_compatible_masters_share_a_key() {
    let (mut doc, mut history) = open("structure.fig");
    let page = doc.pages[0];
    let m1 = run(
        &mut doc,
        &mut history,
        &[shape("FRAME", page, "Card", 0.0, 0.0, 120.0, 60.0)],
    )[0];
    run(&mut doc, &mut history, &[text_layer(m1, 4.0, 4.0, "Label")]);
    let m2 = run(
        &mut doc,
        &mut history,
        &[shape("FRAME", page, "Card", 0.0, 80.0, 120.0, 60.0)],
    )[0];
    run(&mut doc, &mut history, &[text_layer(m2, 4.0, 4.0, "Label")]);
    run(
        &mut doc,
        &mut history,
        &[
            EditOp::CreateComponent { frame: m1 },
            EditOp::CreateComponent { frame: m2 },
        ],
    );
    let mut keys = Txn::new("shared key");
    keys.set_field(m1, "key", Val::Str("shared-compat-key".into()));
    keys.set_field(m2, "key", Val::Str("shared-compat-key".into()));
    doc.apply_txn(&keys).unwrap();

    let inst2 = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: m2,
            parent: page,
            x: 200.0,
            y: 0.0,
        }],
    )[0];
    let dup2 = run(
        &mut doc,
        &mut history,
        &[EditOp::DuplicateNode {
            target: inst2,
            dx: 20.0,
            dy: 0.0,
        }],
    )[0];
    assert_eq!(
        symbol_of(&doc, dup2),
        m2,
        "duplicating an instance of the second compatible master jumped to the first master"
    );
}

#[test]
fn explicit_override_key_is_not_shadowed_by_a_copied_scene_node_with_the_same_guid() {
    // Source master child has explicit `overrideKey == 1:40`, which coincidentally matches the
    // copied scene instance's own node GUID `1:40`. Remapping the instance node ID must not
    // overwrite the `1:40` override path key.
    let mut src = Document::open(fixture("structure.fig")).unwrap();
    let doc_id = Guid(0, 0);
    let page_id = Guid(0, 1);
    let src_master = Guid(1, 1);
    let src_label = Guid(1, 10);
    let shared_guid = Guid(1, 40);

    let src_nodes: Vec<Node> = [
        node_val(doc_id, "DOCUMENT", "Document", None, &[]),
        node_val(page_id, "CANVAS", "Page 1", Some((doc_id, "!")), &[]),
        node_val(
            src_master,
            "SYMBOL",
            "Master",
            Some((page_id, "!")),
            &[("key", Val::Str("shadow-key".into()))],
        ),
        node_val(
            src_label,
            "TEXT",
            "Label",
            Some((src_master, "!")),
            &[
                ("overrideKey", shared_guid.to_val()),
                ("textData", text_data("Base")),
            ],
        ),
        node_val(
            shared_guid,
            "INSTANCE",
            "Use",
            Some((page_id, "\"")),
            &[(
                "symbolData",
                Val::Obj(Obj::from([
                    ("symbolID", src_master.to_val()),
                    (
                        "symbolOverrides",
                        Val::Arr(vec![Val::Obj(Obj::from([
                            ("guidPath", guid_path(&[shared_guid])),
                            ("textData", text_data("Kept")),
                        ]))]),
                    ),
                ])),
            )],
        ),
    ]
    .into_iter()
    .map(|v| Node::from_data(v).unwrap())
    .collect();
    src.set_nodes(src_nodes).unwrap();

    let kiwi = encode_subset_kiwi(&src, &[shared_guid], 1, "shadow").unwrap();

    let mut dst = Document::open(fixture("structure.fig")).unwrap();
    let dst_doc = Guid(0, 0);
    let dst_page = Guid(0, 1);
    let dst_master = Guid(5, 1);
    let dst_label = Guid(5, 20);
    let dst_nodes: Vec<Node> = [
        node_val(dst_doc, "DOCUMENT", "Document", None, &[]),
        node_val(dst_page, "CANVAS", "Page 1", Some((dst_doc, "!")), &[]),
        node_val(
            dst_master,
            "SYMBOL",
            "Master",
            Some((dst_page, "!")),
            &[("key", Val::Str("shadow-key".into()))],
        ),
        node_val(
            dst_label,
            "TEXT",
            "Label",
            Some((dst_master, "!")),
            &[
                ("overrideKey", shared_guid.to_val()),
                ("textData", text_data("Base")),
            ],
        ),
    ]
    .into_iter()
    .map(|v| Node::from_data(v).unwrap())
    .collect();
    dst.set_nodes(dst_nodes).unwrap();

    let (txn, pasted) = paste_kiwi_into(&dst, &kiwi, dst_page, (0.0, 0.0)).unwrap();
    let mut history = History::new(10);
    history.execute(&mut dst, txn).unwrap();
    let pasted_inst = pasted[0];

    assert_ne!(pasted_inst, shared_guid);
    assert_eq!(symbol_of(&dst, pasted_inst), dst_master);
    assert_eq!(
        override_paths(&dst, pasted_inst, &["symbolData", "symbolOverrides"]),
        vec![vec![shared_guid]],
        "copied scene node GUID shadowed the reused master's explicit overrideKey"
    );
    assert_eq!(
        resolve_instance(&dst, pasted_inst).unwrap().nodes[0]
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str),
        Some("Kept")
    );
}
