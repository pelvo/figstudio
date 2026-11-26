//! Regression tests for the component, instance and clipboard-master findings of PR #90:
//! instance text formatting, reused clipboard masters, `InstantiateComponent`, invalid fill
//! hex, `DetachInstance` and `ResetInstanceOverride`.

use std::path::PathBuf;

use figstudio_fig::clipboard::encode_clipboard_html;
use figstudio_fig::edit::fractional;
use figstudio_fig::edit::text_runs::TextStyleRunPatch;
use figstudio_fig::edit::{EditOp, History, Txn, execute_edit_ops, execute_paste_html};
use figstudio_fig::{Document, Guid, Obj, Val, intern};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn open() -> (Document, History) {
    (
        Document::open(fixture("structure.fig")).unwrap(),
        History::new(50),
    )
}

fn try_run(
    doc: &mut Document,
    history: &mut History,
    ops: &[EditOp],
) -> Result<Vec<Guid>, figstudio_fig::FigError> {
    execute_edit_ops(doc, history, ops, None, None, false).map(|r| r.0.created_ids)
}

fn run(doc: &mut Document, history: &mut History, ops: &[EditOp]) -> Vec<Guid> {
    try_run(doc, history, ops).unwrap()
}

fn frame(parent: Guid, w: f64, h: f64) -> EditOp {
    EditOp::CreateShape {
        id: None,
        kind: "FRAME".into(),
        parent,
        name: None,
        x: 0.0,
        y: 0.0,
        width: w,
        height: h,
        corner_radius: 0.0,
        fill_hex: None,
        count: None,
        star_inner_scale: None,
    }
}

fn text(parent: Guid, x: f64, y: f64, characters: &str) -> EditOp {
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

fn bold(start: usize, end: usize, target: Guid) -> EditOp {
    EditOp::StyleTextRange {
        target,
        utf16_start: start,
        utf16_end: end,
        style: TextStyleRunPatch {
            font_style: Some("Bold".into()),
            ..Default::default()
        },
    }
}

fn style_ids(td: &Val) -> Vec<u32> {
    td.get("characterStyleIDs")
        .map(Val::items)
        .unwrap_or_default()
        .iter()
        .map(|v| v.as_u32().unwrap_or(0))
        .collect()
}

fn override_entries(doc: &Document, instance: Guid) -> Vec<Val> {
    doc.node(instance)
        .unwrap()
        .data
        .path(&["symbolData", "symbolOverrides"])
        .map(Val::items)
        .unwrap_or_default()
        .iter()
        .map(|v| v.clone().into_owned())
        .collect()
}

// ---- #3 instance text formatting -------------------------------------------------------

/// A component whose text layer "Label" has its first two characters bold, and one instance.
fn styled_component(doc: &mut Document, history: &mut History) -> (Guid, Guid, Guid) {
    let page = doc.pages[0];
    let master = run(doc, history, &[frame(page, 120.0, 40.0)])[0];
    let label = run(doc, history, &[text(master, 4.0, 4.0, "Label")])[0];
    run(doc, history, &[bold(0, 2, label)]);
    run(doc, history, &[EditOp::CreateComponent { frame: master }]);
    let instance = run(
        doc,
        history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 300.0,
            y: 0.0,
        }],
    )[0];
    (master, label, instance)
}

fn set_chars(doc: &mut Document, history: &mut History, instance: Guid, path: &[Guid], c: &str) {
    run(
        doc,
        history,
        &[EditOp::SetInstanceOverride {
            instance,
            override_path: path.to_vec(),
            characters: Some(c.into()),
            visible: None,
            fill_hex: None,
            opacity: None,
        }],
    );
}

#[test]
fn first_instance_text_override_keeps_the_masters_text_runs() {
    let (mut doc, mut history) = open();
    let (_, label, instance) = styled_component(&mut doc, &mut history);
    let path = vec![doc.node(label).unwrap().override_key()];

    set_chars(&mut doc, &mut history, instance, &path, "Label!");

    let entries = override_entries(&doc, instance);
    let td = entries[0].get("textData").unwrap();
    assert_eq!(td.text("characters"), Some("Label!"));
    let ids = style_ids(td);
    assert!(
        ids.len() >= 2 && ids[0] != 0 && ids[0] == ids[1],
        "the master's bold run was dropped from the override: {ids:?}"
    );
    assert_eq!(td.get("styleOverrideTable").unwrap().items().len(), 1);
}

#[test]
fn repeated_instance_text_override_keeps_its_text_runs() {
    let (mut doc, mut history) = open();
    let (_, label, instance) = styled_component(&mut doc, &mut history);
    let path = vec![doc.node(label).unwrap().override_key()];

    set_chars(&mut doc, &mut history, instance, &path, "Label!");
    set_chars(&mut doc, &mut history, instance, &path, "Label!!");

    let entries = override_entries(&doc, instance);
    let td = entries[0].get("textData").unwrap();
    assert_eq!(td.text("characters"), Some("Label!!"));
    let ids = style_ids(td);
    assert!(
        ids.len() >= 2 && ids[0] != 0 && ids[0] == ids[1],
        "the bold run was dropped by the second override: {ids:?}"
    );
}

#[test]
fn plain_text_edit_keeps_runs_outside_the_changed_span() {
    let (mut doc, mut history) = open();
    let page = doc.pages[0];
    let t = run(
        &mut doc,
        &mut history,
        &[text(page, 0.0, 0.0, "Hello world")],
    )[0];
    run(&mut doc, &mut history, &[bold(0, 5, t)]);

    run(
        &mut doc,
        &mut history,
        &[EditOp::SetText {
            target: t,
            characters: "Hello there".into(),
        }],
    );

    let td = doc.node(t).unwrap().data.get("textData").unwrap().clone();
    assert_eq!(td.text("characters"), Some("Hello there"));
    let ids = style_ids(&td);
    assert!(
        ids.len() >= 5 && ids[..5].iter().all(|&i| i != 0),
        "the bold run on \"Hello\" was dropped: {ids:?}"
    );
    assert!(
        ids.iter().skip(5).all(|&i| i == 0),
        "the edited tail inherited the bold run: {ids:?}"
    );
}

// ---- #10 invalid fill hex --------------------------------------------------------------

#[test]
fn a_text_range_style_with_an_invalid_fill_hex_is_an_error() {
    let (mut doc, mut history) = open();
    let page = doc.pages[0];
    let t = run(&mut doc, &mut history, &[text(page, 0.0, 0.0, "Hello")])[0];
    let before = doc.node(t).unwrap().data.get("textData").unwrap().clone();
    let result = try_run(
        &mut doc,
        &mut history,
        &[EditOp::StyleTextRange {
            target: t,
            utf16_start: 0,
            utf16_end: 3,
            style: TextStyleRunPatch {
                fill_hex: Some("not-a-colour".into()),
                ..Default::default()
            },
        }],
    );
    assert!(result.is_err(), "an invalid fill_hex was silently ignored");
    assert_eq!(doc.node(t).unwrap().data.get("textData"), Some(&before));
}

// ---- #8 InstantiateComponent -----------------------------------------------------------

fn styled_card(doc: &mut Document, history: &mut History) -> Guid {
    let page = doc.pages[0];
    let card = run(
        doc,
        history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("Card".into()),
            x: 0.0,
            y: 0.0,
            width: 160.0,
            height: 90.0,
            corner_radius: 12.0,
            fill_hex: Some("#336699".into()),
            count: None,
            star_inner_scale: None,
        }],
    )[0];
    run(
        doc,
        history,
        &[
            EditOp::SetStroke {
                target: card,
                hex: Some("#112233".into()),
                weight: Some(3.0),
                align: Some("INSIDE".into()),
            },
            EditOp::SetAutoLayout {
                target: card,
                mode: "VERTICAL".into(),
                spacing: Some(8.0),
                counter_spacing: None,
                padding: Some([4.0, 6.0, 8.0, 10.0]),
                primary_sizing: None,
                counter_sizing: None,
                primary_align: None,
                counter_align: None,
                wrap: None,
            },
            EditOp::CreateComponent { frame: card },
        ],
    );
    card
}

/// What an instance draws around the component's children.
const APPEARANCE_FIELDS: [&str; 5] = [
    "fillPaints",
    "strokePaints",
    "strokeWeight",
    "strokeAlign",
    "cornerRadius",
];

/// Auto-Layout container fields a detached frame needs.
const LAYOUT_FIELDS: [&str; 3] = ["stackMode", "stackSpacing", "stackVerticalPadding"];

#[test]
fn an_instance_takes_the_masters_container_appearance() {
    let (mut doc, mut history) = open();
    let card = styled_card(&mut doc, &mut history);
    let page = doc.pages[0];
    let instance = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: card,
            parent: page,
            x: 200.0,
            y: 0.0,
        }],
    )[0];

    let master = doc.node(card).unwrap();
    let inst = doc.node(instance).unwrap();
    for field in APPEARANCE_FIELDS {
        assert!(master.data.get(field).is_some(), "fixture lacks {field}");
        assert_eq!(
            inst.data.get(field),
            master.data.get(field),
            "the instance lost the master's {field}"
        );
    }
}

#[test]
fn a_detached_instance_keeps_the_masters_container_appearance_and_layout() {
    let (mut doc, mut history) = open();
    let card = styled_card(&mut doc, &mut history);
    let page = doc.pages[0];
    let instance = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: card,
            parent: page,
            x: 200.0,
            y: 0.0,
        }],
    )[0];
    let detached = run(
        &mut doc,
        &mut history,
        &[EditOp::DetachInstance { instance }],
    )[0];

    let master = doc.node(card).unwrap();
    let frame = doc.node(detached).unwrap();
    assert_eq!(frame.kind, "FRAME");
    for field in APPEARANCE_FIELDS.into_iter().chain(LAYOUT_FIELDS) {
        assert!(master.data.get(field).is_some(), "fixture lacks {field}");
        assert_eq!(
            frame.data.get(field),
            master.data.get(field),
            "the detached frame lost the master's {field}"
        );
    }
}

// ---- detach and reset ------------------------------------------------------------------

#[test]
fn detach_refuses_an_instance_too_big_to_materialize() {
    let (mut doc, mut history) = open();
    let page = doc.pages[0];
    let master = run(&mut doc, &mut history, &[frame(page, 100.0, 100.0)])[0];
    // One node more than the resolver's cap (`MAX_RESOLVED_NODES`).
    let count = figstudio_fig::resolve::MAX_RESOLVED_NODES + 1;
    let positions = fractional::n_between(None, None, count);
    let mut txn = Txn::new("bulk");
    for (i, position) in positions.into_iter().enumerate() {
        let id = Guid(7_000_000, u32::try_from(i).unwrap() + 1);
        txn.create_node(
            id,
            Val::Obj(Obj::from([
                ("guid", id.to_val()),
                ("type", Val::Enum(intern("RECTANGLE"))),
                ("name", Val::Str("r".into())),
                (
                    "parentIndex",
                    Val::Obj(Obj::from([
                        ("guid", master.to_val()),
                        ("position", Val::Str(position)),
                    ])),
                ),
            ])),
        );
    }
    doc.apply_txn(&txn).unwrap();
    run(
        &mut doc,
        &mut history,
        &[EditOp::CreateComponent { frame: master }],
    );
    let instance = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 0.0,
            y: 0.0,
        }],
    )[0];

    let result = try_run(
        &mut doc,
        &mut history,
        &[EditOp::DetachInstance { instance }],
    );
    assert!(result.is_err(), "a truncated detach replaced the instance");
    assert!(doc.node(instance).is_some(), "the instance was deleted");
}

fn derived(path: &[Guid], marker: f64) -> Val {
    Val::Obj(Obj::from([
        (
            "guidPath",
            Val::Obj(Obj::from([(
                "guids",
                Val::Arr(path.iter().map(|g| g.to_val()).collect()),
            )])),
        ),
        // `size.x` marks the entry: derived entries are schema-checked `NodeChange`s.
        (
            "size",
            Val::Obj(Obj::from([
                ("x", Val::Float(marker)),
                ("y", Val::Float(1.0)),
            ])),
        ),
    ]))
}

fn derived_markers(doc: &Document, instance: Guid) -> Vec<f32> {
    doc.node(instance)
        .unwrap()
        .data
        .get("derivedSymbolData")
        .map(Val::items)
        .unwrap_or_default()
        .iter()
        .filter_map(|e| e.path(&["size", "x"]).and_then(Val::as_f32))
        .collect()
}

#[test]
fn resetting_one_override_path_drops_its_stale_derived_data_only() {
    for field in [None, Some("textData".to_string())] {
        let (mut doc, mut history) = open();
        let (master, label, instance) = styled_component(&mut doc, &mut history);
        let other = run(&mut doc, &mut history, &[text(master, 4.0, 30.0, "Other")])[0];
        let path = vec![doc.node(label).unwrap().override_key()];
        let other_path = vec![doc.node(other).unwrap().override_key()];
        set_chars(&mut doc, &mut history, instance, &path, "Changed");

        let mut seed = Txn::new("seed derived data");
        seed.set_field(
            instance,
            "derivedSymbolData",
            Val::Arr(vec![derived(&path, 1.0), derived(&other_path, 2.0)]),
        );
        doc.apply_txn(&seed).unwrap();

        run(
            &mut doc,
            &mut history,
            &[EditOp::ResetInstanceOverride {
                instance,
                override_path: Some(path.clone()),
                field: field.clone(),
            }],
        );
        assert_eq!(
            derived_markers(&doc, instance),
            vec![2.0_f32],
            "field {field:?}: derived data of the reset path survived"
        );
    }
}

// ---- #4 reused clipboard masters -------------------------------------------------------

/// A keyed component with two text layers ("One" above "Two") and an instance whose "Two"
/// text is overridden. Returns the clipboard html of the instance and the source layer names.
fn keyed_master_html() -> String {
    let (mut src, mut history) = open();
    let page = src.pages[0];
    let master = run(&mut src, &mut history, &[frame(page, 120.0, 80.0)])[0];
    run(&mut src, &mut history, &[text(master, 4.0, 4.0, "One")]);
    let two = run(&mut src, &mut history, &[text(master, 4.0, 40.0, "Two")])[0];
    run(
        &mut src,
        &mut history,
        &[EditOp::CreateComponent { frame: master }],
    );
    let mut key = Txn::new("key");
    key.set_field(master, "key", Val::Str("components-test-key".into()));
    src.apply_txn(&key).unwrap();
    let instance = run(
        &mut src,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 300.0,
            y: 0.0,
        }],
    )[0];
    let path = vec![src.node(two).unwrap().override_key()];
    set_chars(&mut src, &mut history, instance, &path, "Override");
    encode_clipboard_html(&src, &[instance], 1, None, None).unwrap()
}

fn symbol_of(doc: &Document, instance: Guid) -> Guid {
    doc.node(instance)
        .unwrap()
        .data
        .path(&["symbolData", "symbolID"])
        .and_then(Val::as_guid)
        .unwrap()
}

/// The names of the layers an instance's first override entry reaches, outermost first.
fn override_target_name(doc: &Document, instance: Guid) -> Option<String> {
    let entry = override_entries(doc, instance).into_iter().next()?;
    let guid = entry
        .path(&["guidPath", "guids"])?
        .items()
        .first()?
        .as_guid()?;
    let node = doc.node(guid)?;
    assert_eq!(
        node.parent,
        Some(symbol_of(doc, instance)),
        "the override reaches a layer outside the instance's master"
    );
    Some(node.name.clone())
}

fn dst_after_first_paste(html: &str) -> (Document, History, Guid, Guid) {
    let mut dst = Document::open(fixture("corners.fig")).unwrap();
    let page = dst.pages[0];
    let mut history = History::new(50);
    let (first, _) = execute_paste_html(&mut dst, &mut history, html, page, (0.0, 0.0)).unwrap();
    (dst, history, page, first[0])
}

#[test]
fn a_reused_master_with_reordered_same_kind_layers_keeps_overrides_on_their_layer() {
    let html = keyed_master_html();
    let (mut dst, mut history, page, first) = dst_after_first_paste(&html);
    assert_eq!(override_target_name(&dst, first).as_deref(), Some("Two"));

    // Swap the z-order of the pasted master's two layers: same kinds, other pre-order.
    let master = symbol_of(&dst, first);
    let kids = dst.children(master).to_vec();
    assert_eq!(kids.len(), 2);
    let position = |g: Guid| dst.node(g).unwrap().position.clone();
    let (pa, pb) = (position(kids[0]), position(kids[1]));
    let parent_index = |pos: String| {
        Val::Obj(Obj::from([
            ("guid", master.to_val()),
            ("position", Val::Str(pos)),
        ]))
    };
    let mut swap = Txn::new("swap layers");
    swap.set_field(kids[0], "parentIndex", parent_index(pb));
    swap.set_field(kids[1], "parentIndex", parent_index(pa));
    dst.apply_txn(&swap).unwrap();
    assert_ne!(dst.children(master).to_vec(), kids);

    let (second, _) = execute_paste_html(&mut dst, &mut history, &html, page, (0.0, 0.0)).unwrap();
    assert_eq!(symbol_of(&dst, second[0]), master, "the master is reused");
    assert_eq!(
        override_target_name(&dst, second[0]).as_deref(),
        Some("Two"),
        "the override moved to another layer"
    );
}

#[test]
fn a_reused_master_that_no_longer_matches_is_imported_again() {
    let html = keyed_master_html();
    let (mut dst, mut history, page, first) = dst_after_first_paste(&html);

    // The user deleted a layer of the pasted master.
    let master = symbol_of(&dst, first);
    let kids = dst.children(master).to_vec();
    let one = kids
        .iter()
        .copied()
        .find(|&g| dst.node(g).unwrap().name == "One")
        .unwrap();
    let mut del = Txn::new("delete a layer");
    del.delete_node(one);
    dst.apply_txn(&del).unwrap();

    let (second, _) = execute_paste_html(&mut dst, &mut history, &html, page, (0.0, 0.0)).unwrap();
    let new_master = symbol_of(&dst, second[0]);
    assert_ne!(new_master, master, "an incompatible master was reused");
    assert_eq!(dst.children(new_master).len(), 2);
    assert_eq!(
        override_target_name(&dst, second[0]).as_deref(),
        Some("Two"),
        "the override does not reach a layer of the instance's master"
    );
}

// ---- clipboard copy of related roots and reserved guids ---------------------------------

#[test]
fn copying_a_frame_and_its_child_together_pastes_one_intact_frame() {
    let (mut src, mut history) = open();
    let page = src.pages[0];
    let outer = run(&mut src, &mut history, &[frame(page, 100.0, 100.0)])[0];
    let inner = run(&mut src, &mut history, &[frame(outer, 20.0, 20.0)])[0];
    let html = encode_clipboard_html(&src, &[outer, inner], 1, None, None).unwrap();

    let mut dst = Document::open(fixture("corners.fig")).unwrap();
    let dst_page = dst.pages[0];
    let mut h = History::new(5);
    let (pasted, _) = execute_paste_html(&mut dst, &mut h, &html, dst_page, (0.0, 0.0)).unwrap();
    assert_eq!(pasted.len(), 1, "the child was pasted as a second root");
    assert_eq!(dst.children(pasted[0]).len(), 1, "the child left its frame");
}

/// Clipboard bytes of a master and its instance, minus the visible page: a payload whose only
/// canvas is the internal one.
fn payload_without_a_visible_page() -> Vec<u8> {
    let (mut src, mut history) = open();
    let page = src.pages[0];
    let master = run(&mut src, &mut history, &[frame(page, 50.0, 50.0)])[0];
    run(
        &mut src,
        &mut history,
        &[EditOp::CreateComponent { frame: master }],
    );
    let instance = run(
        &mut src,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 100.0,
            y: 0.0,
        }],
    )[0];
    let kiwi = figstudio_fig::clipboard::encode_subset_kiwi(&src, &[instance], 1, "k").unwrap();
    let mut payload = Document::from_bytes(&kiwi).unwrap();
    let visible = payload.pages[0];
    let nodes: Vec<_> = payload
        .nodes
        .iter()
        .filter(|n| n.id != visible)
        .cloned()
        .collect();
    payload.set_nodes(nodes).unwrap();
    assert!(payload.pages.is_empty(), "the payload still has a page");
    payload.encode_canvas().unwrap()
}

#[test]
fn pasting_a_payload_without_a_visible_page_does_not_paste_its_masters_twice() {
    let kiwi = payload_without_a_visible_page();
    let mut dst = Document::open(fixture("corners.fig")).unwrap();
    let page = dst.pages[0];
    let (txn, roots) =
        figstudio_fig::clipboard::paste_kiwi_into(&dst, &kiwi, page, (0.0, 0.0)).unwrap();
    let mut history = History::new(5);
    history.execute(&mut dst, txn).unwrap();
    let symbols = dst
        .nodes
        .iter()
        .filter(|n| n.kind == "SYMBOL" && n.id.0 >= 3_000_000_000)
        .count();
    assert_eq!(symbols, 1, "the master was copied more than once");
    assert!(
        roots
            .iter()
            .all(|&g| dst.node(g).unwrap().kind == "INSTANCE"),
        "a master was pasted onto the page as a root"
    );
}

// ---- follow-up: non-default clipboard guids, multi-version keys, ambiguous overrideKey --

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

#[test]
fn copying_from_a_document_using_guids_0_0_to_0_2_preserves_the_copied_tree_and_master() {
    // Valid source document whose DOCUMENT (10:0) and CANVAS (10:1) do not occupy 0:0..2,
    // while copied scene/dependency nodes legitimately use 0:0, 0:1 and 0:2.
    let mut src = Document::open(fixture("structure.fig")).unwrap();
    let doc_id = Guid(10, 0);
    let page_id = Guid(10, 1);
    let sym_id = Guid(0, 2);
    let child_id = Guid(0, 0);
    let inst_id = Guid(0, 1);

    let sym_data = Val::Obj(Obj::from([
        ("symbolID", sym_id.to_val()),
        (
            "symbolOverrides",
            Val::Arr(vec![Val::Obj(Obj::from([
                (
                    "guidPath",
                    Val::Obj(Obj::from([("guids", Val::Arr(vec![child_id.to_val()]))])),
                ),
                (
                    "textData",
                    Val::Obj(Obj::from([("characters", Val::Str("Changed".into()))])),
                ),
            ]))]),
        ),
    ]));
    let nodes: Vec<_> = [
        node_val(doc_id, "DOCUMENT", "Document", None, &[]),
        node_val(page_id, "CANVAS", "Page 1", Some((doc_id, "!")), &[]),
        node_val(sym_id, "SYMBOL", "Master", Some((page_id, "!")), &[]),
        node_val(
            child_id,
            "TEXT",
            "Label",
            Some((sym_id, "!")),
            &[(
                "textData",
                Val::Obj(Obj::from([("characters", Val::Str("Base".into()))])),
            )],
        ),
        node_val(
            inst_id,
            "INSTANCE",
            "Use",
            Some((page_id, "\"")),
            &[("symbolData", sym_data)],
        ),
    ]
    .into_iter()
    .map(|v| figstudio_fig::Node::from_data(v).unwrap())
    .collect();
    src.set_nodes(nodes).unwrap();

    let html = encode_clipboard_html(&src, &[inst_id], 1, None, None).unwrap();
    let mut dst = Document::open(fixture("corners.fig")).unwrap();
    let dst_page = dst.pages[0];
    let mut history = History::new(10);
    let (pasted, _) =
        execute_paste_html(&mut dst, &mut history, &html, dst_page, (0.0, 0.0)).unwrap();

    assert_eq!(pasted.len(), 1);
    let pasted_inst = pasted[0];
    assert_eq!(dst.node(pasted_inst).unwrap().kind, "INSTANCE");
    let pasted_master = symbol_of(&dst, pasted_inst);
    assert_eq!(dst.node(pasted_master).unwrap().kind, "SYMBOL");
    assert!(
        dst.internal_pages
            .contains(&dst.node(pasted_master).unwrap().parent.unwrap()),
        "master was not placed on the internal canvas"
    );
    assert_eq!(
        override_target_name(&dst, pasted_inst).as_deref(),
        Some("Label")
    );
}

#[test]
fn copying_a_scene_node_at_0_2_with_a_symbol_dependency_keeps_the_internal_canvas() {
    // Valid source document with standard DOCUMENT 0:0 and CANVAS 0:1, no internal canvas,
    // and a copied scene frame at 0:2 containing an instance of a symbol at 1:1.
    let mut src = Document::open(fixture("structure.fig")).unwrap();
    let doc_id = Guid(0, 0);
    let page_id = Guid(0, 1);
    let frame_id = Guid(0, 2);
    let sym_id = Guid(1, 1);
    let inst_id = Guid(1, 2);

    let sym_data = Val::Obj(Obj::from([("symbolID", sym_id.to_val())]));
    let nodes: Vec<_> = [
        node_val(doc_id, "DOCUMENT", "Document", None, &[]),
        node_val(page_id, "CANVAS", "Page 1", Some((doc_id, "!")), &[]),
        node_val(sym_id, "SYMBOL", "Master", Some((page_id, "!")), &[]),
        node_val(frame_id, "FRAME", "Card", Some((page_id, "\"")), &[]),
        node_val(
            inst_id,
            "INSTANCE",
            "Use",
            Some((frame_id, "!")),
            &[("symbolData", sym_data)],
        ),
    ]
    .into_iter()
    .map(|v| figstudio_fig::Node::from_data(v).unwrap())
    .collect();
    src.set_nodes(nodes).unwrap();

    let html = encode_clipboard_html(&src, &[frame_id], 1, None, None).unwrap();
    let mut dst = Document::open(fixture("corners.fig")).unwrap();
    let dst_page = dst.pages[0];
    let mut history = History::new(10);
    let (pasted, _) =
        execute_paste_html(&mut dst, &mut history, &html, dst_page, (0.0, 0.0)).unwrap();

    assert_eq!(pasted.len(), 1);
    let pasted_frame = pasted[0];
    let frame_kids = dst.children(pasted_frame);
    assert_eq!(
        frame_kids.len(),
        1,
        "symbol master leaked into the copied frame"
    );
    let pasted_inst = frame_kids[0];
    assert_eq!(dst.node(pasted_inst).unwrap().kind, "INSTANCE");
    let pasted_master = symbol_of(&dst, pasted_inst);
    assert!(
        dst.internal_pages
            .contains(&dst.node(pasted_master).unwrap().parent.unwrap()),
        "master was not placed on the internal canvas"
    );
}

#[test]
fn pasting_with_multiple_incompatible_masters_sharing_a_key_reuses_each_matching_master() {
    let html = keyed_master_html();
    let (mut dst, mut history, page, first) = dst_after_first_paste(&html);

    // Edit the first imported master so it becomes incompatible with the clipboard payload.
    let edited_master = symbol_of(&dst, first);
    let one = dst
        .children(edited_master)
        .iter()
        .copied()
        .find(|&g| dst.node(g).unwrap().name == "One")
        .unwrap();
    let mut del = Txn::new("delete layer");
    del.delete_node(one);
    dst.apply_txn(&del).unwrap();

    // Pasting the 2-layer payload imports a second master with the same key.
    let (second, _) = execute_paste_html(&mut dst, &mut history, &html, page, (0.0, 0.0)).unwrap();
    let reimported_master = symbol_of(&dst, second[0]);
    assert_ne!(reimported_master, edited_master);
    assert_eq!(
        symbol_of(&dst, first),
        edited_master,
        "first instance aliased"
    );

    // Duplicating `first` must reuse `edited_master` (earlier in `dst.nodes`), not import a 3rd.
    let dup_first = run(
        &mut dst,
        &mut history,
        &[EditOp::DuplicateNode {
            target: first,
            dx: 20.0,
            dy: 0.0,
        }],
    )[0];
    assert_eq!(
        symbol_of(&dst, dup_first),
        edited_master,
        "duplicating the first instance imported a redundant master instead of reusing its own"
    );

    // Pasting the 2-layer payload again must reuse `reimported_master`.
    let (third, _) = execute_paste_html(&mut dst, &mut history, &html, page, (0.0, 0.0)).unwrap();
    assert_eq!(symbol_of(&dst, third[0]), reimported_master);

    let keyed_symbols = dst
        .nodes
        .iter()
        .filter(|n| n.kind == "SYMBOL" && n.data.text("key") == Some("components-test-key"))
        .count();
    assert_eq!(keyed_symbols, 2);
}

#[test]
fn a_master_with_ambiguous_duplicate_override_keys_is_not_reused_by_guessing() {
    let (mut src, mut history) = open();
    let page = src.pages[0];
    let master = run(&mut src, &mut history, &[frame(page, 120.0, 80.0)])[0];
    let first_label = run(&mut src, &mut history, &[text(master, 4.0, 4.0, "Item")])[0];
    let second_label = run(&mut src, &mut history, &[text(master, 4.0, 4.0, "Item")])[0];
    run(
        &mut src,
        &mut history,
        &[EditOp::CreateComponent { frame: master }],
    );
    let dup_key = Guid(9, 1);
    let mut setup = Txn::new("key and duplicate overrideKey");
    setup.set_field(master, "key", Val::Str("dup-override-key-test".into()));
    setup.set_field(first_label, "overrideKey", dup_key.to_val());
    setup.set_field(second_label, "overrideKey", dup_key.to_val());
    src.apply_txn(&setup).unwrap();

    let instance = run(
        &mut src,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 300.0,
            y: 0.0,
        }],
    )[0];
    // Target the first sibling by its node guid.
    set_chars(&mut src, &mut history, instance, &[first_label], "Updated");
    let html = encode_clipboard_html(&src, &[instance], 1, None, None).unwrap();

    let (mut dst, mut dst_history, dst_page, first_pasted) = dst_after_first_paste(&html);
    let first_master = symbol_of(&dst, first_pasted);
    let first_target = override_entries(&dst, first_pasted)[0]
        .path(&["guidPath", "guids"])
        .unwrap()
        .items()[0]
        .as_guid()
        .unwrap();
    assert_eq!(first_target, dst.children(first_master)[0]);

    let (second_pasted, _) =
        execute_paste_html(&mut dst, &mut dst_history, &html, dst_page, (0.0, 0.0)).unwrap();
    let second_master = symbol_of(&dst, second_pasted[0]);
    let second_target = override_entries(&dst, second_pasted[0])[0]
        .path(&["guidPath", "guids"])
        .unwrap()
        .items()[0]
        .as_guid()
        .unwrap();
    assert_ne!(
        second_master, first_master,
        "ambiguous duplicate overrideKey reused the master"
    );
    assert_eq!(
        second_target,
        dst.children(second_master)[0],
        "override swapped onto the second sibling"
    );
}

#[test]
fn a_master_whose_layer_override_key_changed_is_not_reused_by_signature() {
    let (mut src, mut history) = open();
    let page = src.pages[0];
    let master = run(&mut src, &mut history, &[frame(page, 120.0, 40.0)])[0];
    let label = run(&mut src, &mut history, &[text(master, 4.0, 4.0, "Label")])[0];
    run(
        &mut src,
        &mut history,
        &[EditOp::CreateComponent { frame: master }],
    );
    let orig_key = Guid(9, 42);
    let mut setup = Txn::new("key and overrideKey");
    setup.set_field(master, "key", Val::Str("changed-override-key-test".into()));
    setup.set_field(label, "overrideKey", orig_key.to_val());
    src.apply_txn(&setup).unwrap();

    let instance = run(
        &mut src,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 300.0,
            y: 0.0,
        }],
    )[0];
    set_chars(&mut src, &mut history, instance, &[orig_key], "Override");
    let html = encode_clipboard_html(&src, &[instance], 1, None, None).unwrap();

    let (mut dst, mut dst_history, dst_page, first_pasted) = dst_after_first_paste(&html);
    let first_master = symbol_of(&dst, first_pasted);
    let dst_label = dst.children(first_master)[0];

    // Change only the target layer's explicit overrideKey (same kind, name, transform, size).
    let mut tweak = Txn::new("change overrideKey");
    tweak.set_field(dst_label, "overrideKey", Guid(9, 99).to_val());
    dst.apply_txn(&tweak).unwrap();

    let (second_pasted, _) =
        execute_paste_html(&mut dst, &mut dst_history, &html, dst_page, (0.0, 0.0)).unwrap();
    let second_master = symbol_of(&dst, second_pasted[0]);
    assert_ne!(
        second_master, first_master,
        "master with mismatched overrideKey was reused via signature fallback"
    );
    let path = figstudio_fig::resolve::InstancePath::new(vec![second_pasted[0], orig_key]).unwrap();
    let resolved = figstudio_fig::resolve::resolve_path(&dst, &path).unwrap();
    assert_eq!(
        resolved
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str),
        Some("Override")
    );
}

#[test]
fn a_master_with_unique_override_keys_still_reuses_after_layer_rename() {
    let (mut src, mut history) = open();
    let page = src.pages[0];
    let master = run(&mut src, &mut history, &[frame(page, 120.0, 40.0)])[0];
    let label = run(&mut src, &mut history, &[text(master, 4.0, 4.0, "Label")])[0];
    run(
        &mut src,
        &mut history,
        &[EditOp::CreateComponent { frame: master }],
    );
    let key = Guid(9, 77);
    let mut setup = Txn::new("key and unique overrideKey");
    setup.set_field(master, "key", Val::Str("unique-override-key-test".into()));
    setup.set_field(label, "overrideKey", key.to_val());
    src.apply_txn(&setup).unwrap();

    let instance = run(
        &mut src,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 300.0,
            y: 0.0,
        }],
    )[0];
    set_chars(&mut src, &mut history, instance, &[key], "Override");
    let html = encode_clipboard_html(&src, &[instance], 1, None, None).unwrap();

    let (mut dst, mut dst_history, dst_page, first_pasted) = dst_after_first_paste(&html);
    let first_master = symbol_of(&dst, first_pasted);
    let dst_label = dst.children(first_master)[0];

    // Rename the layer in dst while keeping its unique explicit overrideKey.
    run(
        &mut dst,
        &mut dst_history,
        &[EditOp::Rename {
            target: dst_label,
            name: "Renamed".into(),
        }],
    );

    let (second_pasted, _) =
        execute_paste_html(&mut dst, &mut dst_history, &html, dst_page, (0.0, 0.0)).unwrap();
    assert_eq!(
        symbol_of(&dst, second_pasted[0]),
        first_master,
        "unique explicit overrideKey should still allow master reuse"
    );
    let path = figstudio_fig::resolve::InstancePath::new(vec![second_pasted[0], key]).unwrap();
    let resolved = figstudio_fig::resolve::resolve_path(&dst, &path).unwrap();
    assert_eq!(
        resolved
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str),
        Some("Override")
    );
}

// ---- findings 1-2: HUG Auto Layout on instances, detach semantics, top-level swap ------

fn node_size(doc: &Document, id: Guid) -> (f64, f64) {
    let data = &doc.node(id).unwrap().data;
    (
        f64::from(data.path(&["size", "x"]).and_then(Val::as_f32).unwrap()),
        f64::from(data.path(&["size", "y"]).and_then(Val::as_f32).unwrap()),
    )
}

fn solid_rgb(data: &Val) -> Option<(f32, f32, f32)> {
    let c = data.get("fillPaints")?.items().first()?.get("color")?;
    Some((c.num("r")?, c.num("g")?, c.num("b")?))
}

#[test]
fn touching_a_hug_auto_layout_instance_does_not_collapse_its_size() {
    let (mut doc, mut history) = open();
    let page = doc.pages[0];
    let master = run(&mut doc, &mut history, &[frame(page, 120.0, 48.0)])[0];
    let label = run(&mut doc, &mut history, &[text(master, 8.0, 8.0, "Button")])[0];
    run(
        &mut doc,
        &mut history,
        &[
            EditOp::SetAutoLayout {
                target: master,
                mode: "VERTICAL".into(),
                spacing: Some(4.0),
                counter_spacing: None,
                padding: Some([8.0, 8.0, 8.0, 8.0]),
                primary_sizing: Some("HUG".into()),
                counter_sizing: Some("HUG".into()),
                primary_align: None,
                counter_align: None,
                wrap: None,
            },
            EditOp::CreateComponent { frame: master },
        ],
    );

    let inst_standalone = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 200.0,
            y: 0.0,
        }],
    )[0];

    let outer = run(&mut doc, &mut history, &[frame(page, 300.0, 80.0)])[0];
    run(
        &mut doc,
        &mut history,
        &[EditOp::SetAutoLayout {
            target: outer,
            mode: "HORIZONTAL".into(),
            spacing: Some(8.0),
            counter_spacing: None,
            padding: Some([4.0, 4.0, 4.0, 4.0]),
            primary_sizing: Some("HUG".into()),
            counter_sizing: Some("HUG".into()),
            primary_align: None,
            counter_align: None,
            wrap: None,
        }],
    );
    let inst_nested_in_stack = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: outer,
            x: 0.0,
            y: 0.0,
        }],
    )[0];
    let sibling = run(&mut doc, &mut history, &[frame(outer, 40.0, 40.0)])[0];

    // Seed Figma-style HUG Auto Layout fields on both instances.
    let mut seed = Txn::new("seed imported HUG instance fields");
    for inst in [inst_standalone, inst_nested_in_stack] {
        seed.modify(
            inst,
            Obj::from([
                ("stackMode", Val::Enum(intern("VERTICAL"))),
                ("stackPrimarySizing", Val::Enum(intern("RESIZE_TO_FIT"))),
                ("stackCounterSizing", Val::Enum(intern("RESIZE_TO_FIT"))),
                ("stackHorizontalPadding", Val::Float(8.0)),
                ("stackVerticalPadding", Val::Float(8.0)),
                (
                    "size",
                    Val::Obj(Obj::from([
                        ("x", Val::Float(120.0)),
                        ("y", Val::Float(48.0)),
                    ])),
                ),
            ]),
            Vec::new(),
        );
    }
    doc.apply_txn(&seed).unwrap();

    // 1. Touching the standalone instance via SetInstanceOverride must not collapse it.
    let label_key = doc.node(label).unwrap().override_key();
    set_chars(
        &mut doc,
        &mut history,
        inst_standalone,
        &[label_key],
        "Save",
    );
    assert_eq!(
        node_size(&doc, inst_standalone),
        (120.0, 48.0),
        "standalone HUG instance collapsed when touched by SetInstanceOverride"
    );

    // 2. Resizing a sibling in an Auto Layout parent must not collapse the child HUG instance.
    run(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: sibling,
            width: 60.0,
            height: 40.0,
        }],
    );
    assert_eq!(
        node_size(&doc, inst_nested_in_stack),
        (120.0, 48.0),
        "HUG instance inside Auto Layout frame collapsed when ancestor re-solved"
    );
}

#[test]
fn detaching_a_hug_instance_preserves_border_inclusive_hug_and_resized_dimensions() {
    let (mut doc, mut history) = open();
    let page = doc.pages[0];
    let master = run(&mut doc, &mut history, &[frame(page, 100.0, 40.0)])[0];
    let _child = run(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "RECTANGLE".into(),
            parent: master,
            name: Some("Content".into()),
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 24.0,
            corner_radius: 0.0,
            fill_hex: Some("#222222".into()),
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
                hex: Some("#000000".into()),
                weight: Some(4.0),
                align: Some("INSIDE".into()),
            },
            EditOp::SetAutoLayout {
                target: master,
                mode: "HORIZONTAL".into(),
                spacing: Some(0.0),
                counter_spacing: None,
                padding: Some([10.0, 10.0, 10.0, 10.0]),
                primary_sizing: None,
                counter_sizing: None,
                primary_align: None,
                counter_align: None,
                wrap: None,
            },
        ],
    );
    // Use bordersTakeSpace + stackWidth/stackHeight HUG (fields autolayout.rs reads).
    let mut al_extra = Txn::new("seed bordersTakeSpace and stackWidth/stackHeight");
    al_extra.modify(
        master,
        Obj::from([
            ("bordersTakeSpace", Val::Bool(true)),
            ("stackWidth", Val::Enum(intern("RESIZE_TO_FIT"))),
            ("stackHeight", Val::Enum(intern("RESIZE_TO_FIT"))),
        ]),
        Vec::new(),
    );
    doc.apply_txn(&al_extra).unwrap();
    // Solve master size via auto layout: 100 + 2*10 + 2*4 = 128, 24 + 2*10 + 2*4 = 52.
    figstudio_fig::edit::autolayout::apply_auto_layout_to_doc(&mut doc, Some(master)).unwrap();
    assert_eq!(node_size(&doc, master), (128.0, 52.0));
    run(
        &mut doc,
        &mut history,
        &[EditOp::CreateComponent { frame: master }],
    );

    // 1. Unresized instance keeps border-inclusive HUG dimensions on detach.
    let unresized = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 200.0,
            y: 0.0,
        }],
    )[0];
    let detached_unresized = run(
        &mut doc,
        &mut history,
        &[EditOp::DetachInstance {
            instance: unresized,
        }],
    )[0];
    assert_eq!(
        doc.node(detached_unresized)
            .unwrap()
            .data
            .flag("bordersTakeSpace"),
        Some(true),
        "detached frame lost bordersTakeSpace"
    );
    assert_eq!(
        node_size(&doc, detached_unresized),
        (128.0, 52.0),
        "detached unresized HUG frame lost border-inclusive dimensions"
    );

    // 2. Explicitly resized instance (both axes) keeps 240x100 on detach.
    let resized_both = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 400.0,
            y: 0.0,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: resized_both,
            width: 240.0,
            height: 100.0,
        }],
    );
    let detached_resized = run(
        &mut doc,
        &mut history,
        &[EditOp::DetachInstance {
            instance: resized_both,
        }],
    )[0];
    assert_eq!(
        node_size(&doc, detached_resized),
        (240.0, 100.0),
        "detached resized instance snapped back to HUG"
    );

    // 3. Width-only resize fixes width while preserving vertical HUG.
    let resized_w = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master,
            parent: page,
            x: 700.0,
            y: 0.0,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: resized_w,
            width: 240.0,
            height: 52.0,
        }],
    );
    let detached_w = run(
        &mut doc,
        &mut history,
        &[EditOp::DetachInstance {
            instance: resized_w,
        }],
    )[0];
    assert_eq!(node_size(&doc, detached_w), (240.0, 52.0));
    let detached_kid = doc.children(detached_w)[0];
    run(
        &mut doc,
        &mut history,
        &[EditOp::Resize {
            target: detached_kid,
            width: 100.0,
            height: 40.0,
        }],
    );
    // Width stays fixed at 240; height hugs 40 + 2*10 + 2*4 = 68.
    assert_eq!(
        node_size(&doc, detached_w),
        (240.0, 68.0),
        "width-resized detached frame should fix width and keep vertical HUG"
    );
}

#[test]
fn detaching_an_outer_instance_preserves_nested_auto_layout_instance_layout() {
    let (mut doc, mut history) = open();
    let page = doc.pages[0];

    // Inner Auto Layout HUG master with spacing and bordersTakeSpace.
    let inner_master = run(&mut doc, &mut history, &[frame(page, 60.0, 20.0)])[0];
    run(
        &mut doc,
        &mut history,
        &[
            frame(inner_master, 30.0, 20.0),
            frame(inner_master, 30.0, 20.0),
            EditOp::SetStroke {
                target: inner_master,
                hex: Some("#000000".into()),
                weight: Some(2.0),
                align: Some("INSIDE".into()),
            },
            EditOp::SetAutoLayout {
                target: inner_master,
                mode: "HORIZONTAL".into(),
                spacing: Some(12.0),
                counter_spacing: None,
                padding: Some([8.0, 8.0, 8.0, 8.0]),
                primary_sizing: Some("HUG".into()),
                counter_sizing: Some("HUG".into()),
                primary_align: None,
                counter_align: None,
                wrap: None,
            },
        ],
    );
    let mut bts = Txn::new("bordersTakeSpace on inner");
    bts.set_field(inner_master, "bordersTakeSpace", Val::Bool(true));
    doc.apply_txn(&bts).unwrap();
    figstudio_fig::edit::autolayout::apply_auto_layout_to_doc(&mut doc, Some(inner_master))
        .unwrap();
    // 30 + 12 + 30 + 2*8 + 2*2 = 92, 20 + 2*8 + 2*2 = 40.
    assert_eq!(node_size(&doc, inner_master), (92.0, 40.0));
    run(
        &mut doc,
        &mut history,
        &[EditOp::CreateComponent {
            frame: inner_master,
        }],
    );

    // Outer master contains one unresized and one resized instance of inner_master.
    let outer_master = run(&mut doc, &mut history, &[frame(page, 200.0, 150.0)])[0];
    let nested_unresized = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: inner_master,
            parent: outer_master,
            x: 0.0,
            y: 0.0,
        }],
    )[0];
    let nested_resized = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: inner_master,
            parent: outer_master,
            x: 0.0,
            y: 50.0,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[
            EditOp::Resize {
                target: nested_resized,
                width: 160.0,
                height: 60.0,
            },
            EditOp::SetAutoLayout {
                target: outer_master,
                mode: "VERTICAL".into(),
                spacing: Some(10.0),
                counter_spacing: None,
                padding: Some([10.0, 10.0, 10.0, 10.0]),
                primary_sizing: Some("HUG".into()),
                counter_sizing: Some("HUG".into()),
                primary_align: None,
                counter_align: None,
                wrap: None,
            },
            EditOp::CreateComponent {
                frame: outer_master,
            },
        ],
    );
    let _ = nested_unresized;

    let outer_inst = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: outer_master,
            parent: page,
            x: 400.0,
            y: 0.0,
        }],
    )[0];
    let detached_outer = run(
        &mut doc,
        &mut history,
        &[EditOp::DetachInstance {
            instance: outer_inst,
        }],
    )[0];
    let detached_kids = doc.children(detached_outer).to_vec();
    assert_eq!(detached_kids.len(), 2);

    let first_nested = doc.node(detached_kids[0]).unwrap();
    assert_eq!(first_nested.kind, "FRAME");
    assert_eq!(
        first_nested.data.text("stackMode"),
        Some("HORIZONTAL"),
        "nested detached frame lost its master's stackMode"
    );
    assert_eq!(
        first_nested.data.flag("bordersTakeSpace"),
        Some(true),
        "nested detached frame lost its master's bordersTakeSpace"
    );
    assert_eq!(node_size(&doc, detached_kids[0]), (92.0, 40.0));

    let second_nested = doc.node(detached_kids[1]).unwrap();
    assert_eq!(second_nested.kind, "FRAME");
    assert_eq!(
        second_nested.data.text("stackMode"),
        Some("HORIZONTAL"),
        "resized nested detached frame lost its master's stackMode"
    );
    assert_eq!(
        node_size(&doc, detached_kids[1]),
        (160.0, 60.0),
        "resized nested detached frame snapped back to HUG"
    );
}

#[test]
fn swapping_a_top_level_instance_updates_inherited_appearance_and_detaches_cleanly() {
    let (mut doc, mut history) = open();
    let page = doc.pages[0];

    let make_master =
        |doc: &mut Document, history: &mut History, name: &str, hex: &str, radius: f32| -> Guid {
            let f = run(
                doc,
                history,
                &[EditOp::CreateShape {
                    id: None,
                    kind: "FRAME".into(),
                    parent: page,
                    name: Some(name.into()),
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 40.0,
                    corner_radius: radius,
                    fill_hex: Some(hex.into()),
                    count: None,
                    star_inner_scale: None,
                }],
            )[0];
            run(doc, history, &[EditOp::CreateComponent { frame: f }]);
            f
        };

    let master_a = make_master(&mut doc, &mut history, "A", "#00FF00", 0.0);
    let master_b = make_master(&mut doc, &mut history, "B", "#0000FF", 16.0);
    let master_c = make_master(&mut doc, &mut history, "C", "#FF0000", 0.0);

    // 1. Swap A -> B, verify B's appearance, then swap B -> C, verify C's appearance.
    let inst = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master_a,
            parent: page,
            x: 200.0,
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
    let after_b = &doc.node(inst).unwrap().data;
    assert_eq!(
        solid_rgb(after_b),
        Some((0.0, 0.0, 1.0)),
        "swapped instance kept old master A's green fill instead of master B's blue"
    );
    assert_eq!(after_b.num("cornerRadius"), Some(16.0));

    // Detach a swapped instance and confirm it carries B's fill and corner radius cleanly.
    let inst_detach = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master_a,
            parent: page,
            x: 350.0,
            y: 0.0,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[EditOp::SwapInstance {
            instance: inst_detach,
            symbol: master_b,
        }],
    );
    let detached = run(
        &mut doc,
        &mut history,
        &[EditOp::DetachInstance {
            instance: inst_detach,
        }],
    )[0];
    let detached_data = &doc.node(detached).unwrap().data;
    assert_eq!(
        solid_rgb(detached_data),
        Some((0.0, 0.0, 1.0)),
        "detached swapped instance kept master A's green fill"
    );
    assert_eq!(detached_data.num("cornerRadius"), Some(16.0));

    // Chained swap B -> C must drop B's cornerRadius and take C's red fill.
    run(
        &mut doc,
        &mut history,
        &[EditOp::SwapInstance {
            instance: inst,
            symbol: master_c,
        }],
    );
    let after_c = &doc.node(inst).unwrap().data;
    assert_eq!(
        solid_rgb(after_c),
        Some((1.0, 0.0, 0.0)),
        "chained swap B -> C did not take master C's red fill"
    );
    assert_eq!(
        after_c.num("cornerRadius").unwrap_or(0.0),
        0.0,
        "chained swap B -> C kept master B's corner radius"
    );

    // 2. Explicit user fill override on the instance survives a swap while un-overridden radius updates.
    let inst_override = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: master_a,
            parent: page,
            x: 500.0,
            y: 0.0,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[
            EditOp::SetSolidFill {
                target: inst_override,
                hex: Some("#FFFF00".into()),
            },
            EditOp::SwapInstance {
                instance: inst_override,
                symbol: master_b,
            },
        ],
    );
    let after_override_swap = &doc.node(inst_override).unwrap().data;
    assert_eq!(
        solid_rgb(after_override_swap),
        Some((1.0, 1.0, 0.0)),
        "swap clobbered explicit fill override"
    );
    assert_eq!(
        after_override_swap.num("cornerRadius"),
        Some(16.0),
        "swap failed to update non-overridden cornerRadius"
    );

    // 3. ResetInstanceOverride(None, None) after swap reverts inherited look back to master A.
    run(
        &mut doc,
        &mut history,
        &[EditOp::ResetInstanceOverride {
            instance: inst,
            override_path: None,
            field: None,
        }],
    );
    let after_reset = &doc.node(inst).unwrap().data;
    assert_eq!(
        solid_rgb(after_reset),
        Some((0.0, 1.0, 0.0)),
        "ResetInstanceOverride(None, None) did not restore original master A's green fill"
    );

    // 4. SwitchVariant also updates inherited appearance from the old variant to the new variant.
    let group = run(&mut doc, &mut history, &[frame(page, 250.0, 60.0)])[0];
    let var_default = run(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: group,
            name: Some("State=Default".into()),
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 40.0,
            corner_radius: 0.0,
            fill_hex: Some("#00FF00".into()),
            count: None,
            star_inner_scale: None,
        }],
    )[0];
    let var_hover = run(
        &mut doc,
        &mut history,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: group,
            name: Some("State=Hover".into()),
            x: 120.0,
            y: 0.0,
            width: 100.0,
            height: 40.0,
            corner_radius: 14.0,
            fill_hex: Some("#0000FF".into()),
            count: None,
            star_inner_scale: None,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[
            EditOp::CreateComponent { frame: var_default },
            EditOp::CreateComponent { frame: var_hover },
        ],
    );
    let var_inst = run(
        &mut doc,
        &mut history,
        &[EditOp::InstantiateComponent {
            symbol: var_default,
            parent: page,
            x: 650.0,
            y: 0.0,
        }],
    )[0];
    run(
        &mut doc,
        &mut history,
        &[EditOp::SwitchVariant {
            instance: var_inst,
            variants: vec![("State".into(), "Hover".into())],
        }],
    );
    let after_var = &doc.node(var_inst).unwrap().data;
    assert_eq!(
        solid_rgb(after_var),
        Some((0.0, 0.0, 1.0)),
        "SwitchVariant kept old variant's green fill"
    );
    assert_eq!(after_var.num("cornerRadius"), Some(14.0));
}
