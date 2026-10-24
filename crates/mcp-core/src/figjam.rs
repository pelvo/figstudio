//! `get_figjam`: `get_metadata` XML plus FigJam details, with optional small screenshots.
use crate::{
    assets::{MAX_PAYLOAD_BYTES, Payload},
    metadata::{DEFAULT_OUTPUT_BYTES, figjam::Extras, run},
    output::{Block, Output, text_first},
    rendering::Rendering,
    screenshot::render_node,
};
use base64::{Engine, engine::general_purpose::STANDARD};
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

/// The FigJam XML for `node_id`, then one text line and PNG per pictured node when asked.
pub fn figjam(
    doc: &Document,
    node_id: Option<&str>,
    depth: Option<u32>,
    include_images: bool,
    image_scale: Option<f64>,
    rendering: &Rendering,
) -> Output {
    figjam_within(
        doc,
        node_id,
        depth,
        include_images,
        image_scale,
        rendering,
        MAX_PAYLOAD_BYTES,
        DEFAULT_OUTPUT_BYTES,
    )
}

/// [`figjam`] with explicit image payload and XML output budgets in bytes (before base64).
#[expect(clippy::too_many_arguments, reason = "mirrors the MCP tool inputs")]
pub fn figjam_within(
    doc: &Document,
    node_id: Option<&str>,
    depth: Option<u32>,
    include_images: bool,
    image_scale: Option<f64>,
    rendering: &Rendering,
    max_payload: usize,
    max_bytes: usize,
) -> Output {
    let Board {
        xml,
        pictured,
        scale,
    } = match board(doc, node_id, depth, image_scale, max_bytes) {
        Ok(board) => board,
        Err(output) => return output,
    };
    let mut content = vec![Block::text(xml)];
    if include_images {
        let mut payload = Payload::new(max_payload);
        for id in pictured.iter().take(MAX_IMAGES) {
            content.extend(picture(doc, id, scale, rendering, &mut payload));
        }
        let skipped = pictured.len().saturating_sub(MAX_IMAGES);
        if skipped > 0 {
            content.push(Block::text(format!(
                "{skipped} more node(s) not pictured (limit {MAX_IMAGES})"
            )));
        }
    }
    Output::success(text_first(content))
}

/// A text line and PNG for one node, or only a line when it cannot be rendered or no longer
/// fits the reply's image budget.
fn picture(
    doc: &Document,
    id: &str,
    scale: f64,
    rendering: &Rendering,
    payload: &mut Payload,
) -> Vec<Block> {
    if payload.is_full() {
        return vec![Block::text(Payload::omitted(id))];
    }
    let shot = match render_node(doc, id, Some(scale), rendering) {
        Ok(shot) => shot,
        Err(message) => return vec![Block::text(format!("{id}: no image: {message}"))],
    };
    let line = format!("{id}: {}", shot.describe());
    match figstudio_render::encoding::encode_pixmap(shot.frame.pixmap, rendering.format) {
        Ok(png) if payload.take(png.len()) => vec![
            Block::text(line),
            Block::image(STANDARD.encode(png), rendering.format.mime()),
        ],
        Ok(_) => vec![Block::text(Payload::omitted(id))],
        Err(error) => vec![Block::text(format!(
            "{id}: {}",
            crate::error::CoreError::encode(rendering.format.extension(), error)
        ))],
    }
}
