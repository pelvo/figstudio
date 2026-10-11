//! Connectors store no geometry of their own. Their line, label box and label text are
//! generated parts: shapes and glyphs in `derivedImmutableFrameData`, styles in
//! `nodeGenerationData`, each keyed by a one-guid path whose `localID` is the part.

use figstudio_fig::{Node, Obj, Val};
use vello_cpu::kurbo::Affine;

use crate::geom::affine;

/// Part `localID` and the node kind it is drawn as, back to front.
const PARTS: [(u32, &str); 3] = [(0, "VECTOR"), (2, "ROUNDED_RECTANGLE"), (1, "TEXT")];

/// The connector's generated parts as plain nodes, back to front, each with the transform of
/// its parent part (the label text sits inside the label box).
pub(crate) fn parts(connector: &Node) -> Vec<(Node, Affine)> {
    let mut out = Vec::new();
    for (local, kind) in PARTS {
        let mut data = Obj::new();
        for field in ["nodeGenerationData", "derivedImmutableFrameData"] {
            let list = connector.data.path(&[field, "overrides"]);
            let part = list.map(Val::items).unwrap_or_default().iter().find(|o| {
                matches!(o.path(&["guidPath", "guids"]).map(Val::items),
                    Some([g]) if g.get("localID").and_then(Val::as_u32) == Some(local))
            });
            if let Some(Val::Obj(o)) = part {
                for (k, v) in o.iter().filter(|(k, _)| *k != "guidPath") {
                    data.insert(k, v.clone());
                }
            }
        }
        if !data.is_empty() {
            out.push(Node {
                id: connector.id,
                parent: Some(connector.id),
                position: String::new(),
                kind: kind.into(),
                name: String::new(),
                data: Val::Obj(data),
            });
        }
    }
    let label_box = out
        .iter()
        .find(|n| n.kind == "ROUNDED_RECTANGLE")
        .map_or(Affine::IDENTITY, |n| affine(n.data.get("transform")));
    out.into_iter()
        .map(|n| {
            let parent = if n.kind == "TEXT" {
                label_box
            } else {
                Affine::IDENTITY
            };
            (n, parent)
        })
        .collect()
}
