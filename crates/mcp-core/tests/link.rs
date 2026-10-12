use figstudio_fig::{Guid, resolve::InstancePath};
use figstudio_mcp_core::{
    link::{FileHint, Link, format_node_link, format_region_link, parse_link},
    node_id::{NodeId, parse_node_id},
};

fn path() -> NodeId {
    NodeId::InstancePath(InstancePath::new(vec![Guid(1, 2), Guid(3, 4), Guid(5, 6)]).unwrap())
}

const KEYS: [&str; 7] = [
    "structure",
    "My File v2",
    "a/b?c#d%e",
    "日本語 ファイル",
    "50%",
    "dots.and-dash_under~tilde",
    "pa&th=1;x",
];

#[test]
fn node_links_round_trip_for_odd_keys_and_instance_paths() {
    for key in KEYS {
        for node in [NodeId::Node(Guid(1, 42)), NodeId::Node(Guid(0, 0)), path()] {
            let text = format_node_link(key, &node);
            let link = parse_link(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(link.file, Some(FileHint::Exact(key.to_owned())), "{text}");
            assert_eq!(link.node, Some(node.clone()), "{text}");
            assert_eq!(parse_node_id(&text).unwrap(), node, "{text}");
        }
    }
}

#[test]
fn region_links_round_trip_with_two_decimals() {
    for key in KEYS {
        let text = format_region_link(key, Guid(0, 1), [-10.5, 0.0, 100.126, 20.0]).unwrap();
        let link = parse_link(&text).unwrap();
        assert_eq!(
            link,
            Link {
                file: Some(FileHint::Exact(key.to_owned())),
                node: None,
                page: Some(Guid(0, 1)),
                rect: Some([-10.5, 0.0, 100.13, 20.0]),
            },
            "{text}"
        );
    }
}

#[test]
fn spaces_are_percent_encoded_in_the_host() {
    assert_eq!(
        format_node_link("My File", &NodeId::Node(Guid(1, 2))),
        "figstudio://My%20File?node-id=1-2"
    );
}

#[test]
fn figma_links_keep_working() {
    let link = parse_link("https://www.figma.com/design/AbC123/My-File?node-id=1-42&t=x").unwrap();
    assert_eq!(
        link.file,
        Some(FileHint::Figma {
            key: "AbC123".into(),
            slug: Some("My-File".into())
        })
    );
    assert_eq!(link.node, Some(NodeId::Node(Guid(1, 42))));
    let link = parse_link("https://figma.com/board/K?node-id=I1%3A2%3B3%3A4").unwrap();
    assert_eq!(
        link.node.unwrap(),
        NodeId::InstancePath(InstancePath::new(vec![Guid(1, 2), Guid(3, 4)]).unwrap())
    );
    // No file part: only the node counts.
    let link = parse_link("https://www.figma.com/?node-id=1-2").unwrap();
    assert_eq!(
        (link.file, link.node),
        (None, Some(NodeId::Node(Guid(1, 2))))
    );
}

#[test]
fn bad_links_are_rejected() {
    for input in [
        "figstudio://",
        "figstudio://sections",
        "figstudio://sections?",
        "figstudio://sections?node-id=1-2&node-id=3-4",
        "figstudio://sections?node-id=1-2&rect=1,2,3,4",
        "figstudio://sections?node-id=1-2&page=0-1&rect=1,2,3,4",
        "figstudio://sections?page=0-1",
        "figstudio://sections?rect=1,2,3,4",
        "figstudio://sections?page=0-1&rect=1,2,3",
        "figstudio://sections?page=0-1&rect=1,2,3,4,5",
        "figstudio://sections?page=0-1&rect=1,2,NaN,4",
        "figstudio://sections?page=0-1&rect=1,2,inf,4",
        "figstudio://sections?page=0-1&rect=1,2,0,4",
        "figstudio://sections?page=0-1&rect=1,2,3,-4",
        "figstudio://sections?page=0-1&rect=1e8,2,3,4",
        "figstudio://sections?node-id=bad",
        "figstudio://sections?node-id=1-2&other=1",
        "figstudio://sections/extra?node-id=1-2",
        "figstudio://user@sections?node-id=1-2",
        "figstudio://sections:80?node-id=1-2",
        "file:///etc/passwd",
        "http://figma.com.evil/design/K?node-id=1-2",
        "https://example.com/design/K?node-id=1-2",
        "https://figma.com/design/K?node-id=1-2&node-id=3-4",
        "not a link",
    ] {
        let error = parse_link(input)
            .err()
            .unwrap_or_else(|| panic!("accepted {input}"));
        assert!(error.is_invalid_link(), "{input}");
    }
}

#[test]
fn a_region_link_is_not_a_node_id() {
    let text = format_region_link("sections", Guid(0, 1), [0.0, 0.0, 5.0, 5.0]).unwrap();
    assert!(parse_node_id(&text).unwrap_err().is_invalid_node_id());
    assert!(
        parse_node_id("figstudio://sections")
            .unwrap_err()
            .is_invalid_node_id()
    );
}

#[test]
fn unformattable_regions_are_errors() {
    for rect in [
        [0.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 0.004, 1.0],
        [f64::NAN, 0.0, 1.0, 1.0],
        [0.0, 0.0, f64::INFINITY, 1.0],
        [2e7, 0.0, 1.0, 1.0],
    ] {
        assert!(
            format_region_link("k", Guid(0, 1), rect).is_err(),
            "{rect:?}"
        );
    }
}

#[test]
fn old_inputs_still_parse() {
    for (input, id) in [
        ("1:2", Guid(1, 2)),
        ("1-2", Guid(1, 2)),
        ("10:200", Guid(10, 200)),
    ] {
        assert_eq!(parse_node_id(input).unwrap(), NodeId::Node(id));
    }
}
