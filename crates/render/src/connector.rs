//! Connectors store no geometry of their own. Their line, label box and label text are
//! generated parts: shapes and glyphs in `derivedImmutableFrameData`, styles in
//! `nodeGenerationData`, each keyed by a one-guid path whose `localID` is the part.

use figstudio_fig::{Node, Obj, Val};
use vello_cpu::kurbo::Affine;

use crate::geom::affine;

/// Part `localID` and the node kind it is drawn as, back to front.
const PARTS: [(u32, &str); 3] = [(0, "VECTOR"), (2, "ROUNDED_RECTANGLE"), (1, "TEXT")];

/// Borrowed view of a connector part's two override layers (`derived` overrides `generated`).
#[derive(Clone, Copy)]
pub(crate) struct PartRef<'a> {
    pub(crate) generated: Option<&'a Val>,
    pub(crate) derived: Option<&'a Val>,
}

impl<'a> PartRef<'a> {
    pub(crate) fn get(self, key: &str) -> Option<&'a Val> {
        self.derived
            .and_then(|o| o.get(key))
            .or_else(|| self.generated.and_then(|o| o.get(key)))
    }

    pub(crate) fn flag(self, key: &str) -> Option<bool> {
        self.get(key).and_then(Val::as_bool)
    }

    pub(crate) fn is_non_empty(self) -> bool {
        [self.generated, self.derived]
            .into_iter()
            .flatten()
            .any(|layer| matches!(layer, Val::Obj(o) if o.keys().any(|k| k != "guidPath")))
    }

    /// Materializes a merged `Val::Obj`. Only called when rendering or on a hit candidate.
    pub(crate) fn to_val(self) -> Val {
        let mut out = Obj::new();
        for layer in [self.generated, self.derived].into_iter().flatten() {
            if let Val::Obj(o) = layer {
                for (k, v) in o.iter().filter(|(k, _)| *k != "guidPath") {
                    out.insert(k, v.clone());
                }
            }
        }
        Val::Obj(out)
    }
}

/// Zero-clone single-pass lookup of one override layer (`derivedImmutableFrameData` or
/// `nodeGenerationData`) indexed by part slot: `[0: VECTOR, 1: ROUNDED_RECTANGLE, 2: TEXT]`.
pub(crate) fn layer_slots<'a>(data: &'a Val, field: &str) -> [Option<&'a Val>; 3] {
    let mut dst = [None; 3];
    let Some(overrides) = data.path(&[field, "overrides"]).map(Val::items) else {
        return dst;
    };
    for o in overrides {
        let slot = match o.path(&["guidPath", "guids"]).map(Val::items) {
            Some([g]) => match g.as_guid().map(|g| g.1) {
                Some(0) => Some(0),
                Some(2) => Some(1),
                Some(1) => Some(2),
                _ => None,
            },
            _ => None,
        };
        if let Some(slot) = slot
            && dst[slot].is_none()
        {
            dst[slot] = Some(o);
        }
    }
    dst
}

/// The connector's generated parts as plain nodes, back to front, each with the transform of
/// its parent part (the label text sits inside the label box).
pub(crate) fn parts(connector: &Node) -> Vec<(Node, Affine)> {
    let generated = layer_slots(&connector.data, "nodeGenerationData");
    let derived = layer_slots(&connector.data, "derivedImmutableFrameData");
    let label_box = PartRef {
        generated: generated[1],
        derived: derived[1],
    };
    let label_tf = if label_box.is_non_empty() {
        affine(label_box.get("transform"))
    } else {
        Affine::IDENTITY
    };
    PARTS
        .iter()
        .enumerate()
        .filter_map(|(i, (_, kind))| {
            let part = PartRef {
                generated: generated[i],
                derived: derived[i],
            };
            part.is_non_empty().then(|| {
                let parent = if *kind == "TEXT" {
                    label_tf
                } else {
                    Affine::IDENTITY
                };
                (
                    Node {
                        id: connector.id,
                        parent: Some(connector.id),
                        position: String::new(),
                        kind: (*kind).into(),
                        name: String::new(),
                        data: part.to_val(),
                    },
                    parent,
                )
            })
        })
        .collect()
}
