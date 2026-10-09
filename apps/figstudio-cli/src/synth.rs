//! `synth`: builds a large stress-test file by copying the first page's layers many times.
//!
//! Copies get new ids (a fresh session id per copy, same local ids), are laid out in a grid,
//! and can be spread over extra pages. Instances keep pointing at the original components.

use std::collections::HashMap;

use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_render::content_bounds;

/// Session ids at and above this are used for copies; real files use small session ids.
const FIRST_SESSION: u32 = 3_000_000_000;

fn guid_val(id: Guid) -> Val {
    id.to_val()
}

fn set(data: &mut Val, key: &str, value: Val) {
    data.set(key, value);
}

fn parent_val(parent: Guid, position: &str) -> Val {
    Val::Obj(Obj::from([
        ("guid", parent.to_val()),
        ("position", Val::Str(position.into())),
    ]))
}

fn shift(data: &mut Val, dx: f64, dy: f64) {
    let Val::Obj(m) = data else { return };
    let Some(Val::Obj(tf)) = m.get_mut("transform") else {
        return;
    };
    for (key, d) in [("m02", dx), ("m12", dy)] {
        let v = tf.get(key).and_then(Val::as_f32).map_or(0.0, f64::from);
        tf.insert(key, Val::Float(f64::from((v + d) as f32)));
    }
}

/// Pre-order list of every node below `root`.
fn subtree(doc: &Document, root: Guid) -> Vec<Guid> {
    let mut out = Vec::new();
    let mut stack: Vec<Guid> = doc.children(root).iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        out.push(id);
        stack.extend(doc.children(id).iter().rev());
    }
    out
}

pub fn synth(mut doc: Document, copies: usize, pages: usize) -> Result<Document, String> {
    let page0 = *doc.pages.first().ok_or("source has no pages")?;
    let source = subtree(&doc, page0);
    if source.is_empty() {
        return Err("the first page is empty".into());
    }
    let b = content_bounds(&doc, page0).ok_or("the first page has nothing visible")?;
    let (w, h) = (b.width().max(1.0), b.height().max(1.0));
    let gap = w.max(h) * 0.1 + 40.0;
    let cols = (copies as f64).sqrt().ceil().max(1.0) as usize;
    let needed = (copies * pages).saturating_sub(1);
    if needed > (u32::MAX - FIRST_SESSION) as usize {
        return Err("too many copies".into());
    }

    let mut nodes = doc.nodes.clone();
    nodes.reserve(needed * source.len() + pages);
    let page_node = doc.node(page0).ok_or("missing page")?.clone();
    let mut session = FIRST_SESSION;
    for p in 0..pages {
        let target = if p == 0 {
            page0
        } else {
            // A new page after the original one.
            let id = Guid(FIRST_SESSION - 1, p as u32);
            let mut data = page_node.data.clone();
            set(&mut data, "guid", guid_val(id));
            set(
                &mut data,
                "name",
                Val::Str(format!("Stress page {}", p + 1)),
            );
            let parent = page_node.parent.ok_or("page has no parent")?;
            set(
                &mut data,
                "parentIndex",
                parent_val(parent, &format!("{}~{p:06}", page_node.position)),
            );
            nodes.push(Node::from_data(data).ok_or("bad page")?);
            id
        };
        for k in 0..copies {
            if p == 0 && k == 0 {
                continue; // the original layers
            }
            let (col, row) = (k % cols, k / cols);
            let (dx, dy) = (col as f64 * (w + gap), row as f64 * (h + gap));
            let map: HashMap<Guid, Guid> =
                source.iter().map(|id| (*id, Guid(session, id.1))).collect();
            session += 1;
            for id in &source {
                let node = doc.node(*id).ok_or("missing node")?;
                let mut data = node.data.clone();
                set(&mut data, "guid", guid_val(map[id]));
                let parent = node.parent.ok_or("node has no parent")?;
                let new_parent = if parent == page0 {
                    target
                } else {
                    map[&parent]
                };
                set(
                    &mut data,
                    "parentIndex",
                    parent_val(new_parent, &node.position),
                );
                if parent == page0 {
                    shift(&mut data, dx, dy);
                }
                nodes.push(Node::from_data(data).ok_or("bad node")?);
            }
        }
    }
    doc.set_nodes(nodes).map_err(|e| e.to_string())?;
    Ok(doc)
}
