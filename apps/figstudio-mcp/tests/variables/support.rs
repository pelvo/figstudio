use super::*;
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/fixtures/{name}.fig"))
}
pub fn obj<const N: usize>(fields: [(&str, Val); N]) -> Val {
    Val::Obj(Obj::from(fields))
}
pub fn id(n: u32) -> Val {
    Guid(9, n).to_val()
}
pub fn alias(n: u32) -> Val {
    obj([("value", obj([("alias", obj([("guid", id(n))]))]))])
}
pub fn node(n: u32, kind: &str, name: &str, fields: &[(&str, Val)]) -> Node {
    let mut d = obj([
        ("guid", id(n)),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str(name.into())),
        ("parentIndex", obj([("guid", id(1))])),
    ]);
    for (key, value) in fields {
        d.set(key, value.clone());
    }
    let mut node = Node::from_data(d).unwrap();
    if kind == "VARIABLE" || kind == "VARIABLE_SET" || node.data.get("styleType").is_some() {
        node.parent = None;
    }
    node
}
pub fn variable(n: u32, name: &str, values: Vec<Val>) -> Node {
    node(
        n,
        "VARIABLE",
        name,
        &[
            ("variableSetID", obj([("guid", id(10))])),
            (
                "variableDataValues",
                obj([(
                    "entries",
                    Val::Arr(
                        values
                            .into_iter()
                            .enumerate()
                            .map(|(i, v)| obj([("modeID", id(20 + i as u32)), ("variableData", v)]))
                            .collect(),
                    ),
                )]),
            ),
        ],
    )
}
pub fn color() -> Val {
    obj([
        ("r", Val::Float(1.0)),
        ("g", Val::Float(0.0)),
        ("b", Val::Float(0.0)),
        ("a", Val::Float(0.5)),
    ])
}
pub fn document() -> Document {
    let mut doc = Document::open(fixture("structure")).unwrap();
    let mut root = node(
        1,
        "FRAME",
        "root",
        &[
            ("styleIdForFill", obj([("guid", id(30))])),
            ("inheritTextStyleID", id(31)),
            ("styleIdForEffect", obj([("guid", id(32))])),
            ("variableData", alias(6)),
            ("unresolved", obj([("variableData", alias(7))])),
            ("missing", obj([("colorVar", alias(99))])),
            ("styleIdForGrid", obj([("guid", id(33))])),
            ("styleIdForStrokeFill", obj([("guid", id(34))])),
            ("textBinding", obj([("variableData", alias(8))])),
            ("boolBinding", obj([("variableData", alias(9))])),
            (
                "variableConsumptionMap",
                obj([(
                    "entries",
                    Val::Arr(vec![obj([
                        ("nodeField", Val::Int(1)),
                        ("variableData", alias(2)),
                    ])]),
                )]),
            ),
        ],
    );
    root.parent = None;
    root.data
        .get_mut("parentIndex")
        .unwrap()
        .set("guid", Guid(0, 0).to_val());
    let stop = obj([("position", Val::Float(0.0)), ("colorVar", alias(3))]);
    let paint = obj([
        ("type", Val::Str("GRADIENT_LINEAR".into())),
        ("stops", Val::Arr(vec![stop])),
    ]);
    let fragment = obj([("fillPaints", Val::Arr(vec![paint]))]);
    let text_data = obj([("styleOverrideTable", Val::Arr(vec![fragment]))]);
    let nodes = vec![
        root,
        node(
            10,
            "VARIABLE_SET",
            "Theme",
            &[
                ("variableSetDefaultModeID", id(21)),
                (
                    "variableSetModes",
                    Val::Arr(vec![
                        obj([("id", id(20)), ("name", Val::Str("Light".into()))]),
                        obj([("id", id(21)), ("name", Val::Str("Dark".into()))]),
                    ]),
                ),
            ],
        ),
        variable(
            2,
            "Spacing",
            vec![
                obj([("value", obj([("floatValue", Val::Float(4.0))]))]),
                obj([("value", obj([("floatValue", Val::Float(8.0))]))]),
            ],
        ),
        variable(
            3,
            "Accent",
            vec![
                obj([("value", obj([("colorValue", color())]))]),
                obj([("value", obj([("colorValue", color())]))]),
            ],
        ),
        variable(4, "Unused", vec![alias(3)]),
        variable(6, "Alias", vec![alias(2), alias(2)]),
        variable(7, "Cycle", vec![alias(7)]),
        variable(
            8,
            "Label",
            vec![obj([(
                "value",
                obj([("textValue", Val::Str("sample".into()))]),
            )])],
        ),
        variable(
            9,
            "Enabled",
            vec![obj([("value", obj([("boolValue", Val::Bool(true))]))])],
        ),
        node(
            33,
            "FRAME",
            "Grid",
            &[
                ("styleType", Val::Enum("GRID")),
                (
                    "layoutGrids",
                    Val::Arr(vec![obj([
                        ("pattern", Val::Enum("COLUMNS")),
                        ("count", Val::Int(12)),
                    ])]),
                ),
            ],
        ),
        node(
            34,
            "RECTANGLE",
            "Gradient",
            &[
                ("styleType", Val::Enum("STROKE")),
                (
                    "fillPaints",
                    Val::Arr(vec![obj([
                        ("type", Val::Enum("GRADIENT_LINEAR")),
                        (
                            "stops",
                            Val::Arr(vec![obj([
                                ("position", Val::Float(0.0)),
                                ("color", color()),
                            ])]),
                        ),
                    ])]),
                ),
            ],
        ),
        node(
            30,
            "RECTANGLE",
            "Surface",
            &[
                ("styleType", Val::Str("FILL".into())),
                (
                    "fillPaints",
                    Val::Arr(vec![obj([
                        ("type", Val::Str("SOLID".into())),
                        ("color", color()),
                    ])]),
                ),
            ],
        ),
        node(
            31,
            "TEXT",
            "Body",
            &[
                ("styleType", Val::Str("TEXT".into())),
                (
                    "fontName",
                    obj([
                        ("family", Val::Str("Sans".into())),
                        ("style", Val::Str("Regular".into())),
                    ]),
                ),
                ("fontSize", Val::Float(16.0)),
                ("fontWeight", Val::Int(400)),
                (
                    "lineHeight",
                    obj([("value", Val::Float(24.0)), ("units", Val::Enum("PIXELS"))]),
                ),
                (
                    "letterSpacing",
                    obj([("value", Val::Float(0.0)), ("units", Val::Enum("PIXELS"))]),
                ),
            ],
        ),
        node(
            32,
            "RECTANGLE",
            "Shadow",
            &[
                ("styleType", Val::Str("EFFECT".into())),
                (
                    "effects",
                    Val::Arr(vec![obj([
                        ("type", Val::Str("DROP_SHADOW".into())),
                        ("color", color()),
                        (
                            "offset",
                            obj([("x", Val::Float(1.0)), ("y", Val::Float(2.0))]),
                        ),
                        ("radius", Val::Float(3.0)),
                        ("spread", Val::Float(0.0)),
                    ])]),
                ),
            ],
        ),
        node(5, "TEXT", "child", &[("textData", text_data)]),
    ];
    doc.set_nodes(nodes).unwrap();
    doc
}
