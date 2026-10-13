//! `synth`: builds a large stress-test file by copying the first page's layers many times.
//!
//! Copies get new ids (a fresh session id per copy, same local ids), are laid out in a grid,
//! and can be spread over extra pages. Instances keep pointing at the original components.

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow, bail};
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_render::content_bounds;

/// Session ids at and above this are used for copies; real files use small session ids.
const FIRST_SESSION: u32 = 3_000_000_000;

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

pub fn synth(mut doc: Document, copies: usize, pages: usize) -> Result<Document> {
    let page0 = *doc.pages.first().context("source has no pages")?;
    let source = subtree(&doc, page0);
    if source.is_empty() {
        bail!("the first page is empty");
    }
    let b = content_bounds(&doc, page0).context("the first page has nothing visible")?;
    let (w, h) = (b.width().max(1.0), b.height().max(1.0));
    let gap = w.max(h) * 0.1 + 40.0;
    let cols = (copies as f64).sqrt().ceil().max(1.0) as usize;
    let needed = (copies * pages).saturating_sub(1);
    if needed > (u32::MAX - FIRST_SESSION) as usize {
        bail!("too many copies");
    }

    let mut nodes = doc.nodes.clone();
    nodes.reserve(needed * source.len() + pages);
    let page_node = doc.node(page0).context("missing page")?.clone();
    let mut session = FIRST_SESSION;
    for p in 0..pages {
        let target = if p == 0 {
            page0
        } else {
            // A new page after the original one.
            let id = Guid(FIRST_SESSION - 1, p as u32);
            let mut data = page_node.data.clone();
            data.set("guid", id.to_val());
            data.set("name", Val::Str(format!("Stress page {}", p + 1)));
            let parent = page_node.parent.context("page has no parent")?;
            data.set(
                "parentIndex",
                parent_val(parent, &format!("{}~{p:06}", page_node.position)),
            );
            nodes.push(Node::from_data(data).context("bad page")?);
            id
        };
        for k in 0..copies {
            if p == 0 && k == 0 {
                continue; // the original layers
            }
            let (col, row) = (k % cols, k / cols);
            let (dx, dy) = (col as f64 * (w + gap), row as f64 * (h + gap));
            let map: HashMap<Guid, Guid> = source
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    let local =
                        u32::try_from(i + 1).map_err(|_| anyhow!("too many source nodes"))?;
                    Ok((*id, Guid(session, local)))
                })
                .collect::<Result<_>>()?;
            session += 1;
            for id in &source {
                let node = doc.node(*id).context("missing node")?;
                let mut data = node.data.clone();
                data.set("guid", map[id].to_val());
                let parent = node.parent.context("node has no parent")?;
                let new_parent = if parent == page0 {
                    target
                } else {
                    map[&parent]
                };
                data.set("parentIndex", parent_val(new_parent, &node.position));
                if parent == page0 {
                    shift(&mut data, dx, dy);
                }
                nodes.push(Node::from_data(data).context("bad node")?);
            }
        }
    }
    doc.set_nodes(nodes).map_err(|e| anyhow!("{e}"))?;
    Ok(doc)
}
