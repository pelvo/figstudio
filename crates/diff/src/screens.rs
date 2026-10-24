//! Screen discovery and deterministic identity/name-size matching.
use figstudio_fig::{Document, Guid};
use figstudio_render::{absolute_transform, geom::size, kurbo::Point};
use std::collections::BTreeSet;

pub(crate) use crate::MatchedBy as MatchBy;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScreenPair {
    pub old: Option<Guid>,
    pub new: Option<Guid>,
    pub matched_by: Option<MatchBy>,
}

pub(crate) fn discover(doc: &Document, page: Guid) -> Vec<Guid> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stack = vec![page];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        for &child in doc.children(id) {
            let Some(node) = doc.node(child) else {
                continue;
            };
            match node.kind.as_str() {
                "SECTION" => stack.push(child),
                // Component sets are SYMBOLs with isStateGroup in decoded .fig files.
                "FRAME" | "SYMBOL" | "INSTANCE" | "GROUP" | "COMPONENT_SET" => found.push(child),
                _ => {}
            }
        }
    }
    found.sort();
    found
}

pub(crate) fn match_screens(old: &Document, new: &Document, page: Guid) -> Vec<ScreenPair> {
    let a = discover(old, page);
    let b = discover(new, page);
    let mut used_a = BTreeSet::new();
    let mut used_b = BTreeSet::new();
    let mut pairs = Vec::new();
    for &id in &a {
        if b.binary_search(&id).is_ok() {
            used_a.insert(id);
            used_b.insert(id);
            pairs.push(ScreenPair {
                old: Some(id),
                new: Some(id),
                matched_by: Some(MatchBy::Id),
            });
        }
    }
    let mut candidates = Vec::new();
    for &i in a.iter().filter(|i| !used_a.contains(i)) {
        let Some(an) = old.node(i) else {
            continue;
        };
        let (aw, ah) = size(&an.data);
        for &j in b.iter().filter(|j| !used_b.contains(j)) {
            let Some(bn) = new.node(j) else {
                continue;
            };
            let (bw, bh) = size(&bn.data);
            if an.kind == bn.kind
                && an.name == bn.name
                && (aw - bw).abs() <= 1.0
                && (ah - bh).abs() <= 1.0
            {
                let ap = absolute_transform(old, i) * Point::ZERO;
                let bp = absolute_transform(new, j) * Point::ZERO;
                candidates.push((ap.distance_squared(bp), i, j));
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    for (_, i, j) in candidates {
        if !used_a.contains(&i) && !used_b.contains(&j) {
            used_a.insert(i);
            used_b.insert(j);
            pairs.push(ScreenPair {
                old: Some(i),
                new: Some(j),
                matched_by: Some(MatchBy::NameSize),
            });
        }
    }
    pairs.extend(
        a.into_iter()
            .filter(|i| !used_a.contains(i))
            .map(|i| ScreenPair {
                old: Some(i),
                new: None,
                matched_by: None,
            }),
    );
    pairs.extend(
        b.into_iter()
            .filter(|i| !used_b.contains(i))
            .map(|i| ScreenPair {
                old: None,
                new: Some(i),
                matched_by: None,
            }),
    );
    pairs.sort_by_key(|p| (p.old.is_none(), p.old, p.new));
    pairs
}

#[cfg(test)]
#[path = "screens_tests.rs"]
mod tests;
