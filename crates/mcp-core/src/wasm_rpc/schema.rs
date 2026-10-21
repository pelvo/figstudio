use serde_json::{Value, json};

/// Browser tool schemas, checked against the native rmcp server by its drift test.
pub fn tool_list() -> Vec<Value> {
    let string = json!({"type":["string","null"]});
    let integer = json!({"type":["integer","null"],"minimum":0});
    let number = json!({"type":["number","null"]});
    let boolean = json!({"type":["boolean","null"]});
    let strings = json!({"type":["array","null"],"items":{"type":"string"}});
    let format = json!({"type":["string","null"],"enum":["png","webp",null]});
    let asset_format = json!({"type":["string","null"],"enum":["png","webp","jpg",null]});
    let node = vec![("nodeId", string.clone()), ("fileKey", string.clone())];
    let mut metadata = node.clone();
    metadata.push(("depth", integer.clone()));
    let mut figjam = metadata.clone();
    figjam.extend([
        ("includeImages", boolean.clone()),
        ("imageScale", number.clone()),
        ("format", format.clone()),
    ]);
    let mut design = node.clone();
    design.extend([
        ("clientFrameworks", string.clone()),
        ("clientLanguages", string.clone()),
        ("maxNodes", integer.clone()),
        ("format", format.clone()),
    ]);
    vec![
        tool("list_files", "List files open in the browser.", vec![], &[]),
        tool(
            "list_open_tabs",
            "List open tabs and their active page and selection.",
            vec![],
            &[],
        ),
        tool(
            "get_selection",
            "Describe the selected node in an open tab.",
            vec![("fileKey", string.clone())],
            &[],
        ),
        tool(
            "reveal_node",
            "Select and show a node, or the outer instance of an instance path.",
            vec![
                ("nodeId", json!({"type":"string"})),
                ("fileKey", string.clone()),
            ],
            &["nodeId"],
        ),
        tool(
            "get_metadata",
            "Sparse XML of pages or a node subtree.",
            metadata,
            &[],
        ),
        tool(
            "get_screenshot",
            "Render a node or region, format png (default) or webp. Defaults to selection; scale 0.01 to 4.",
            [
                node.clone(),
                vec![("scale", number.clone()), ("format", format.clone())],
            ]
            .concat(),
            &[],
        ),
        tool(
            "get_nodes_in_region",
            "List nodes in a region link or page + rect [x,y,width,height].",
            vec![
                ("fileKey", string.clone()),
                ("region", string.clone()),
                ("page", string.clone()),
                (
                    "rect",
                    json!({"type":["array","null"],"items":{"type":"number"},"minItems":4,"maxItems":4}),
                ),
                ("limit", integer.clone()),
            ],
            &[],
        ),
        tool(
            "get_variable_defs",
            "Variables and styles used by a subtree; defaults to selection.",
            node.clone(),
            &[],
        ),
        tool(
            "get_design_context",
            "React + Tailwind reference code with inline assets, format png (default) or webp; defaults to selection.",
            design,
            &[],
        ),
        tool(
            "download_assets",
            "Render up to 20 nodes, format png (default), webp or jpg (flattened onto white), or return original image fills. Defaults to selection.",
            vec![
                ("nodeIds", strings.clone()),
                ("fileKey", string.clone()),
                ("format", asset_format),
                ("scale", number),
                ("original", boolean),
            ],
            &[],
        ),
        tool(
            "get_figjam",
            "Sparse board XML, optionally with images, format png (default) or webp.",
            figjam,
            &[],
        ),
        tool(
            "search_design_system",
            "Search local components, styles and variables; no team libraries.",
            vec![
                ("query", json!({"type":"string"})),
                ("kinds", strings),
                ("fileKey", string.clone()),
            ],
            &["query"],
        ),
        tool(
            "get_libraries",
            "Bounded inventory of this file's own components and tokens.",
            vec![("fileKey", string)],
            &[],
        ),
        tool(
            "get_context_for_code_connect",
            "Local component properties and variants; no cloud bindings.",
            node,
            &[],
        ),
    ]
}

fn tool(name: &str, description: &str, fields: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let properties: serde_json::Map<_, _> =
        fields.into_iter().map(|(k, v)| (k.into(), v)).collect();
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required}})
}
