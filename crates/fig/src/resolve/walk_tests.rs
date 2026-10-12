//! The walk queues children lazily.
use super::*;
use crate::Obj;
use std::cell::Cell;

thread_local! {
    /// Largest walk stack seen, to check children are queued one at a time.
    pub(super) static PEAK: Cell<usize> = const { Cell::new(0) };
}

fn node(n: u32, parent: u32, kind: &str) -> Node {
    let guid = |n: u32| Guid(9, n).to_val();
    let mut data = Obj::new();
    data.insert("guid", guid(n));
    data.insert("type", Val::Str(kind.into()));
    let mut parent_index = Obj::new();
    parent_index.insert("guid", guid(parent));
    data.insert("parentIndex", Val::Obj(parent_index));
    if kind == "INSTANCE" {
        let mut symbol = Obj::new();
        symbol.insert("symbolID", guid(2));
        data.insert("symbolData", Val::Obj(symbol));
    }
    Node::from_data(Val::Obj(data)).unwrap()
}

#[test]
fn a_wide_component_is_not_queued_past_the_node_cap() {
    let mut doc = Document::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    let mut nodes = vec![node(2, 0, "SYMBOL"), node(1, 0, "INSTANCE")];
    nodes.extend((10..60_010).map(|n| node(n, 2, "RECTANGLE")));
    doc.set_nodes(nodes).unwrap();
    let definitions = Arc::new(definitions(&doc));
    for budget in [0, 3] {
        PEAK.with(|peak| peak.set(0));
        let walk = walk_instance(&doc, &definitions, Guid(9, 1), None, budget).unwrap();
        assert_eq!(walk.nodes.len(), budget);
        assert!(walk.truncated);
        assert!(
            PEAK.with(Cell::get) < 8,
            "stack grew to {}",
            PEAK.with(Cell::get)
        );
    }
}
