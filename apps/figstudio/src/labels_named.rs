//! Page-order label candidates, with transforms and section subtree ranges resolved once.
use figstudio_fig::{Document, Guid};
use figstudio_render::{geom, kurbo::Affine};

#[derive(Debug)]
pub(super) struct Named {
    pub id: Guid,
    pub world: Affine,
    /// Exclusive end of this section's subtree; ordinary frames contain no candidates.
    pub end: usize,
    /// Index of the enclosing section candidate, whose fill is under this label.
    pub parent: Option<usize>,
}

pub(super) fn collect(doc: &Document, page: Guid) -> Vec<Named> {
    let mut out: Vec<Named> = Vec::new();
    // Exit markers close the ancestor range without walking parents for each collision.
    let mut stack: Vec<_> = doc
        .children(page)
        .iter()
        .rev()
        .map(|&id| (id, Affine::IDENTITY, None::<usize>, None::<usize>))
        .collect();
    while let Some((id, parent, exit, section_parent)) = stack.pop() {
        if let Some(index) = exit {
            let end = out.len();
            out[index].end = end;
            continue;
        }
        let Some(node) = doc.node(id) else { continue };
        let section = node.kind == "SECTION";
        if !(section || geom::is_container(&node.kind)) || node.data.flag("visible") == Some(false)
        {
            continue;
        }
        let index = out.len();
        let world = parent * geom::affine(node.data.get("transform"));
        out.push(Named {
            id,
            world,
            end: index + 1,
            parent: section_parent,
        });
        if section {
            stack.push((id, world, Some(index), None));
            stack.extend(
                doc.children(id)
                    .iter()
                    .rev()
                    .map(|&id| (id, world, None, Some(index))),
            );
        }
    }
    out
}
