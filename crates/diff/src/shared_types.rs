use figstudio_fig::Guid;
use serde::{Deserialize, Serialize};

/// A repeated visual change. Its picture is the changed component itself when that component
/// exists in both files (`component_view`), else the crops of `occurrences[0]`, which is the
/// occurrence with the largest region (most context).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedChange {
    /// Zero-based index into DiffReport::shared_changes.
    pub id: usize,
    pub width: u32,
    pub height: u32,
    /// Number of distinct screens, not number of region occurrences.
    pub count: usize,
    pub old_png: Option<String>,
    pub new_png: Option<String>,
    pub occurrences: Vec<SharedOccurrence>,
    /// The changed component all occurrences come from; absent for pixel-identical groups.
    #[serde(default)]
    pub component: Option<SharedComponent>,
    /// The pictures show the component itself, not an occurrence's crop.
    #[serde(default)]
    pub component_view: bool,
    /// Pixel size of the pictures (2x); they may show more than `width` x `height`.
    #[serde(default)]
    pub image_width: Option<u32>,
    #[serde(default)]
    pub image_height: Option<u32>,
}

/// The component (SYMBOL) whose change several screens share.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedComponent {
    /// The shown variant when the component belongs to a set (else the component itself).
    pub id: Guid,
    pub name: String,
    /// The COMPONENT_SET whose variants all changed alike; its name titles the card.
    #[serde(default)]
    pub set: Option<SharedSet>,
    /// Every changed variant of the set, the shown one first, then document order.
    #[serde(default)]
    pub variants: Vec<SharedVariant>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedSet {
    pub id: Guid,
    pub name: String,
}

/// One variant of a changed component set. `old_png`/`new_png` are set only for variants that
/// look different from the shown one (and from each other); the shown variant uses the card's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedVariant {
    pub id: Guid,
    pub name: String,
    /// Distinct screens where this variant's instances changed.
    pub count: usize,
    #[serde(default)]
    pub old_png: Option<String>,
    #[serde(default)]
    pub new_png: Option<String>,
    #[serde(default)]
    pub image_width: Option<u32>,
    #[serde(default)]
    pub image_height: Option<u32>,
}

/// Zero-based report coordinates of one grouped region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SharedOccurrence {
    pub page_index: usize,
    pub screen_index: usize,
    pub region_index: usize,
}
