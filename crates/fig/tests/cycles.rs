//! Parent-link cycles in a file must be refused at load, before any walk can recurse forever.

use std::path::PathBuf;

use figstudio_fig::{Document, Guid, Node, Val};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(format!("{name}.fig"))
}

/// Points `child`'s parent at `parent` in the public `nodes` list. `to_bytes` writes that list
/// as it is, so this builds the crafted file without going through the validated `set_nodes`.
fn reparent(doc: &mut Document, child: Guid, parent: Guid) {
    let n = doc.nodes.iter_mut().find(|n| n.id == child).unwrap();
    let mut pi = n.data.get("parentIndex").cloned().unwrap();
    if let Val::Obj(m) = &mut pi {
        m.insert("guid", parent.to_val());
    }
    n.data.set("parentIndex", pi);
    *n = Node::from_data(n.data.clone()).unwrap();
}

/// A file whose page is a child of its own first child: page -> a -> page.
fn cyclic_bytes() -> Vec<u8> {
    let mut doc = Document::open(fixture("structure")).unwrap();
    let page = doc.pages[0];
    let a = doc.children(page)[0];
    reparent(&mut doc, page, a);
    doc.to_bytes().unwrap()
}

#[test]
fn a_parent_cycle_is_refused_on_load() {
    match Document::from_bytes(&cyclic_bytes()) {
        Ok(_) => panic!("a parent cycle was accepted"),
        Err(e) => assert!(e.to_string().contains("cycle"), "{e}"),
    }
}
