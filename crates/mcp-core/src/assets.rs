//! `download_assets`: PNG or JPEG renders of nodes, or the original bytes of their images.
use crate::{
    error::CoreError,
    output::{Block, Output, text_first},
    rendering::Rendering,
    screenshot::{SCALE_RANGE, render_node, svg_node},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::Document;
use image::{ExtendedColorType, codecs::jpeg::JpegEncoder};
use std::collections::HashSet;

mod originals;
use originals::original_images;

/// Most nodes one call may name.
pub const MAX_NODES: usize = 20;
/// Most original images one call returns. Claude Code truncates a tool reply at 25k tokens and
/// each image costs about 1.5k, so 12 images leave room for the text lines.
pub const MAX_ORIGINALS: usize = 12;
/// Raw bytes (before base64) of images one call may return, rendered and originals together.
/// Claude Code disconnects a stdio server that writes more than 16 MB without a JSON-RPC line
/// break; 10 MiB raw is about 13.3 MiB of base64 plus JSON, under that.
pub const MAX_PAYLOAD_BYTES: usize = 10 * 1024 * 1024;
/// Raw bytes of one image: 3.75 MB, which is 5 MB of base64 (the per-image limit on Amazon
/// Bedrock and Google Vertex, the stricter of the Claude platforms; the direct API allows
/// 10 MB). Base64 of n raw bytes is 4n/3 characters, so 3 750 000 bytes make exactly 5 MB.
/// A larger image is reported as omitted with its size.
pub const MAX_IMAGE_BYTES: usize = 3_750_000;
/// [`MAX_IMAGE_BYTES`] as printed in replies.
pub const IMAGE_CAP_LABEL: &str = "3.75 MB";
/// [`MAX_PAYLOAD_BYTES`] as printed in replies.
pub const PAYLOAD_LABEL: &str = "10 MiB";
const JPEG_QUALITY: u8 = 90;

/// Output format of a rendered asset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AssetFormat {
    /// Lossless, keeps transparency.
    #[default]
    Png,
    /// Lossless WebP, keeps transparency.
    Webp,
    /// Lossy; transparent areas are flattened onto white.
    Jpg,
    /// Vector paths (text as outlines, images embedded as PNG), returned as text.
    Svg,
}

/// Renders or extracts the assets for `node_ids`; the result is an error only when the
/// request is invalid or every node failed.
pub fn download(
    doc: &Document,
    node_ids: &[String],
    format: AssetFormat,
    scale: Option<f64>,
    original: bool,
    rendering: &Rendering,
) -> Output {
    download_within(
        doc,
        node_ids,
        format,
        scale,
        original,
        rendering,
        MAX_PAYLOAD_BYTES,
    )
}

/// [`download`] with an explicit payload budget in bytes (before base64).
pub fn download_within(
    doc: &Document,
    node_ids: &[String],
    format: AssetFormat,
    scale: Option<f64>,
    original: bool,
    rendering: &Rendering,
    max_payload: usize,
) -> Output {
    if node_ids.is_empty() || node_ids.len() > MAX_NODES {
        return Output::fault(format!("nodeIds must hold 1 to {MAX_NODES} ids"));
    }
    if scale.is_some_and(|s| !SCALE_RANGE.contains(&s)) {
        return Output::fault("scale must be between 0.01 and 4");
    }
    let mut content = Vec::new();
    let mut failed = 0;
    let mut budget = Budget::new(max_payload);
    for id in node_ids {
        let outcome = if original {
            original_images(doc, id, rendering, &mut budget)
        } else {
            rendered(doc, id, format, scale, rendering, &mut budget.payload)
        };
        match outcome {
            Ok(items) => content.extend(items),
            Err(error) => {
                failed += 1;
                content.push(Block::text(format!("{id}: error: {error}")));
            }
        }
    }
    let all_failed = failed == node_ids.len();
    let content = text_first(content);
    if all_failed {
        Output::error(content)
    } else {
        Output::success(content)
    }
}

/// Raw bytes (before base64) still allowed in one reply.
#[derive(Debug)]
pub struct Payload(usize);

impl Payload {
    pub fn new(max: usize) -> Self {
        Self(max)
    }

    /// Whether anything more fits at all.
    pub fn is_full(&self) -> bool {
        self.0 == 0
    }

    /// Whether `bytes` more would fit.
    pub fn fits(&self, bytes: usize) -> bool {
        bytes <= self.0
    }

    /// Reserves `bytes`; `false` (and nothing taken) when they do not fit.
    pub fn take(&mut self, bytes: usize) -> bool {
        match self.0.checked_sub(bytes) {
            Some(rest) => {
                self.0 = rest;
                true
            }
            None => false,
        }
    }

    /// The text of an item that did not fit the reply's budget.
    pub fn omitted(id: &str) -> String {
        format!("{id}: omitted, the reply holds at most {PAYLOAD_LABEL} of image data")
    }

    /// The text of an image over [`MAX_IMAGE_BYTES`].
    pub fn too_large(id: &str, bytes: usize) -> String {
        format!(
            "{id}: omitted, image of {bytes} bytes is over the {IMAGE_CAP_LABEL} per-image limit"
        )
    }
}

/// What one call may still return: image bytes, original images and which were sent.
pub(crate) struct Budget {
    pub(crate) payload: Payload,
    pub(crate) originals: usize,
    pub(crate) sent: HashSet<String>,
}

impl Budget {
    pub(crate) fn new(max_payload: usize) -> Self {
        Self {
            payload: Payload::new(max_payload),
            originals: MAX_ORIGINALS,
            sent: HashSet::new(),
        }
    }
}

/// A text line and the encoded image of one node.
fn rendered(
    doc: &Document,
    id: &str,
    format: AssetFormat,
    scale: Option<f64>,
    rendering: &Rendering,
    payload: &mut Payload,
) -> Result<Vec<Block>, CoreError> {
    if payload.is_full() {
        return Err(CoreError::limit(Payload::omitted(id)));
    }
    if format == AssetFormat::Svg {
        return svg(doc, id, scale, rendering, payload);
    }
    let shot = render_node(doc, id, scale, rendering)?;
    let mut text = format!("{id}: {}", shot.describe());
    let pixmap = shot.frame.pixmap;
    let (bytes, mime) = match format {
        // SVG returned above.
        AssetFormat::Png | AssetFormat::Svg => (
            pixmap.into_png().map_err(|e| CoreError::encode("PNG", e))?,
            "image/png",
        ),
        AssetFormat::Webp => (
            figstudio_render::encoding::encode_pixmap(
                pixmap,
                figstudio_render::encoding::ImageFormat::Webp,
            )
            .map_err(|e| CoreError::encode("WebP", e))?,
            "image/webp",
        ),
        AssetFormat::Jpg => {
            text.push_str("\nflattened onto white");
            let (w, h) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
            (jpeg(pixmap.data_as_u8_slice(), w, h)?, "image/jpeg")
        }
    };
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(CoreError::limit(Payload::too_large(id, bytes.len())));
    }
    if !payload.take(bytes.len()) {
        return Err(CoreError::limit(Payload::omitted(id)));
    }
    Ok(vec![
        Block::text(text),
        Block::image(STANDARD.encode(bytes), mime),
    ])
}

/// A text line and the SVG markup (a second text block) of one node.
fn svg(
    doc: &Document,
    id: &str,
    scale: Option<f64>,
    rendering: &Rendering,
    payload: &mut Payload,
) -> Result<Vec<Block>, CoreError> {
    let shot = svg_node(doc, id, scale, rendering, MAX_IMAGE_BYTES)?;
    let bytes = shot.svg.markup.len();
    if !payload.take(bytes) {
        return Err(CoreError::limit(Payload::omitted(id)));
    }
    Ok(vec![
        Block::text(format!("{id}: {}", shot.describe())),
        Block::text(shot.svg.markup),
    ])
}

/// JPEG of premultiplied RGBA8 `pixels` composited over white.
fn jpeg(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, CoreError> {
    let rgb: Vec<u8> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            let hole = 255 - p[3];
            [
                p[0].saturating_add(hole),
                p[1].saturating_add(hole),
                p[2].saturating_add(hole),
            ]
        })
        .collect();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode(&rgb, width, height, ExtendedColorType::Rgb8)
        .map_err(|e| CoreError::encode("JPEG", e))?;
    Ok(out)
}
