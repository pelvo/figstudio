use figstudio_fig::{
    Document, Guid, Node, Obj, Val,
    tokens::{Tokens, Value},
};

fn id(n: u32) -> Guid {
    Guid(1, n)
}
fn obj<const N: usize>(fields: &[(&str, Val); N]) -> Val {
    Val::Obj(Obj::from(fields.clone()))
}
fn data(value: Val) -> Val {
    obj(&[("value", value)])
}
fn color() -> Val {
    obj(&[
        ("r", Val::Float(1.0)),
        ("g", Val::Float(0.5)),
        ("b", Val::Float(0.0)),
        ("a", Val::Float(1.0)),
    ])
}
fn alias(n: u32) -> Val {
    data(obj(&[("alias", obj(&[("guid", id(n).to_val())]))]))
}
fn node(n: u32, kind: &str, fields: &[(&str, Val)]) -> Node {
    let mut v = obj(&[
        ("guid", id(n).to_val()),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str(format!("token-{n}"))),
    ]);
    for (k, value) in fields {
        v.set(k, value.clone());
    }
    Node::from_data(v).unwrap()
}
fn variable(n: u32, values: Vec<Val>) -> Node {
    node(
        n,
        "VARIABLE",
        &[
            ("variableSetID", obj(&[("guid", id(10).to_val())])),
            (
                "variableDataValues",
                obj(&[(
                    "entries",
                    Val::Arr(
                        values
                            .into_iter()
                            .enumerate()
                            .map(|(i, v)| {
                                obj(&[("modeID", id(20 + i as u32).to_val()), ("variableData", v)])
                            })
                            .collect(),
                    ),
                )]),
            ),
        ],
    )
}
fn document(nodes: Vec<Node>) -> Document {
    let mut doc = Document::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    doc.set_nodes(nodes).unwrap();
    doc
}
#[test]
fn modes_aliases_and_missing_targets() {
    let doc = document(vec![
        node(
            10,
            "VARIABLE_SET",
            &[
                (
                    "variableSetModes",
                    Val::Arr(vec![
                        obj(&[("id", id(20).to_val()), ("name", Val::Str("Light".into()))]),
                        obj(&[("id", id(21).to_val()), ("name", Val::Str("Dark".into()))]),
                    ]),
                ),
                ("variableSetDefaultModeID", id(21).to_val()),
            ],
        ),
        variable(
            1,
            vec![
                data(obj(&[("colorValue", color())])),
                data(obj(&[("floatValue", Val::Float(12.5))])),
            ],
        ),
        variable(2, vec![alias(1), alias(1)]),
        variable(3, vec![alias(2)]),
        variable(4, vec![alias(5)]),
        variable(5, vec![alias(4)]),
        variable(6, vec![alias(99)]),
        variable(
            7,
            vec![data(obj(&[("textValue", Val::Str("hello".into()))]))],
        ),
        variable(8, vec![data(obj(&[("boolValue", Val::Bool(true))]))]),
    ]);
    let t = Tokens::new(&doc);
    let c = t.collection(id(10)).unwrap();
    assert_eq!(
        c.modes.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["Light", "Dark"]
    );
    assert_eq!(c.default_mode, Some(id(21)));
    assert_eq!(t.variables().count(), 8);
    let v = t.variable(&id(1).to_val()).unwrap();
    assert_eq!(v.collection, Some(id(10)));
    assert_eq!(
        t.resolve(&v.values[0].data, Some(id(20))),
        Some(Value::Color(color()))
    );
    assert_eq!(
        t.resolve(
            &t.variable(&id(3).to_val()).unwrap().values[0].data,
            Some(id(21))
        ),
        Some(Value::Float(12.5))
    );
    for n in [4, 6] {
        assert_eq!(
            t.resolve(&t.variable(&id(n).to_val()).unwrap().values[0].data, None),
            None
        );
    }
    assert_eq!(
        t.resolve(&t.variable(&id(7).to_val()).unwrap().values[0].data, None),
        Some(Value::String("hello".into()))
    );
    assert_eq!(
        t.resolve(&t.variable(&id(8).to_val()).unwrap().values[0].data, None),
        Some(Value::Bool(true))
    );
}
#[test]
fn first_entry_and_depth_limit_are_preserved() {
    let mut target = variable(1, vec![data(obj(&[("colorValue", color())]))]);
    target.data.set("key", Val::Str("local-asset".into()));
    target.data.set("version", Val::Str("v1".into()));
    let mut malformed = variable(2, vec![alias(1)]);
    malformed.data.set(
        "variableDataValues",
        obj(&[(
            "entries",
            Val::Arr(vec![obj(&[]), obj(&[("variableData", alias(1))])]),
        )]),
    );
    let doc = document(vec![target, malformed]);
    let t = Tokens::new(&doc);
    let reference = obj(&[(
        "assetRef",
        obj(&[
            ("key", Val::Str("local-asset".into())),
            ("version", Val::Str("v1".into())),
        ]),
    )]);
    assert_eq!(t.variable(&reference).unwrap().id, id(1));
    let binding = data(obj(&[("alias", reference)]));
    assert_eq!(t.resolve(&binding, None), Some(Value::Color(color())));
    assert_eq!(t.resolve(&alias(2), None), None);
    assert_eq!(
        t.resolve_at(&binding, None, figstudio_fig::tokens::MAX_ALIAS_DEPTH),
        None
    );
}

#[test]
fn style_payload_and_asset_lookup() {
    let paints = Val::Arr(vec![obj(&[
        ("type", Val::Str("SOLID".into())),
        ("color", color()),
    ])]);
    let doc = document(vec![
        node(
            1,
            "RECTANGLE",
            &[
                ("styleType", Val::Str("FILL".into())),
                ("fillPaints", paints.clone()),
                ("key", Val::Str("asset".into())),
                ("version", Val::Str("v1".into())),
            ],
        ),
        node(
            2,
            "TEXT",
            &[
                ("styleType", Val::Str("TEXT".into())),
                ("fontSize", Val::Float(16.0)),
                (
                    "fontName",
                    obj(&[
                        ("family", Val::Str("Sans".into())),
                        ("style", Val::Str("Regular".into())),
                    ]),
                ),
            ],
        ),
    ]);
    let t = Tokens::new(&doc);
    let reference = obj(&[(
        "assetRef",
        obj(&[
            ("key", Val::Str("asset".into())),
            ("version", Val::Str("v1".into())),
        ]),
    )]);
    assert_eq!(t.style(&reference).unwrap().paints.as_ref(), Some(&paints));
    assert_eq!(
        t.style(&id(2).to_val()).unwrap().data.num("fontSize"),
        Some(16.0)
    );
    assert_eq!(
        t.style(&id(2).to_val()).unwrap().kind,
        figstudio_fig::tokens::StyleKind::Text
    );
}

#[test]
fn unknown_style_kinds_and_font_weight_inference() {
    use figstudio_fig::tokens::{StyleKind, font_weight};
    let doc = document(vec![node(
        1,
        "FRAME",
        &[("styleType", Val::Str("FUTURE".into()))],
    )]);
    assert_eq!(
        Tokens::new(&doc).style(&id(1).to_val()).unwrap().kind,
        StyleKind::Other("FUTURE".into())
    );
    for (style, weight) in [
        ("Hairline Italic", 100),
        ("Ultra-Light", 200),
        ("Light", 300),
        ("Regular", 400),
        ("Medium", 500),
        ("Demi Bold", 600),
        ("Bold", 700),
        ("ExtraBold", 800),
        ("Heavy", 900),
    ] {
        assert_eq!(font_weight(style), weight, "{style}");
    }
}
