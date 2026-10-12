//! `get_figjam` text: `get_metadata` XML plus FigJam details and the nodes worth a picture.
//! The pictures themselves are rendered by the caller.
use crate::{
    metadata::{figjam::Extras, run},
    output::Output,
};
use figstudio_fig::Document;

/// Most screenshots one call returns.
pub const MAX_IMAGES: usize = 20;
/// Largest `scale` for those screenshots; they are previews, not exports.
pub const MAX_IMAGE_SCALE: f64 = 0.5;
const DEFAULT_IMAGE_SCALE: f64 = 0.25;

/// A FigJam board listing.
#[derive(Debug)]
pub struct Board {
    /// The metadata XML with FigJam details.
    pub xml: String,
    /// Ids of nodes worth a screenshot, in document order (uncapped).
    pub pictured: Vec<String>,
    /// The validated screenshot scale.
    pub scale: f64,
}

/// The FigJam XML for `node_id`. `Err` is the finished tool reply: a bad `image_scale`, a bad id or a
/// write fault.
pub fn board(
    doc: &Document,
    node_id: Option<&str>,
    depth: Option<u32>,
    image_scale: Option<f64>,
    max_bytes: usize,
) -> Result<Board, Output> {
    let scale = image_scale.unwrap_or(DEFAULT_IMAGE_SCALE);
    if !(0.01..=MAX_IMAGE_SCALE).contains(&scale) {
        return Err(Output::fault("imageScale must be between 0.01 and 0.5"));
    }
    let (xml, extras) = run(doc, node_id, depth, Some(Extras::default()), max_bytes)?;
    Ok(Board {
        xml,
        pictured: extras.unwrap_or_default().pictured,
        scale,
    })
}
