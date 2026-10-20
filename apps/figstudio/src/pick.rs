//! Selection context is the selected layer's parent, not the deepest painted layer.

use figstudio_fig::{Document, Guid};

pub fn click(doc: &Document, chain: &[Guid], selected: Option<Guid>, deep: bool) -> Option<Guid> {
    if deep {
        return chain.last().copied();
    }
    if let Some(selected) = selected {
        if chain.contains(&selected) {
            return Some(selected);
        }
        if let Some(parent) = doc.node(selected).and_then(|n| n.parent)
            && let Some(index) = chain.iter().position(|id| *id == parent)
        {
            return chain.get(index + 1).copied().or(Some(parent));
        }
    }
    let first = *chain.first()?;
    let frame = doc.node(first).is_some_and(|n| n.kind == "FRAME");
    chain.get(usize::from(frame && chain.len() > 1)).copied()
}

pub fn deeper(chain: &[Guid], selected: Option<Guid>) -> Option<Guid> {
    selected
        .and_then(|id| chain.iter().position(|candidate| *candidate == id))
        .and_then(|index| chain.get(index + 1).copied())
        .or(selected)
}

pub fn child(doc: &Document, selected: Guid) -> Option<Guid> {
    doc.children(selected).iter().copied().find(|id| {
        doc.node(*id).is_some_and(|n| {
            n.data.flag("visible") != Some(false) && n.data.flag("mask") != Some(true)
        })
    })
}

pub fn parent(doc: &Document, page: Guid, selected: Guid) -> Option<Guid> {
    doc.node(selected)?.parent.filter(|id| *id != page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_and_drill_are_distinct() {
        let chain = [Guid(2, 1), Guid(2, 2), Guid(2, 3)];
        assert_eq!(deeper(&chain, Some(chain[0])), Some(chain[1]));
        assert_eq!(deeper(&chain, Some(chain[2])), Some(chain[2]));
        assert_eq!(deeper(&[], None), None);
    }
}
