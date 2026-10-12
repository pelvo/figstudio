//! `download_assets`: PNG or JPEG renders of nodes, or the original bytes of their images.
use crate::{
    FigstudioServer,
    tools::{
        rendering::Rendering,
        screenshot::{SCALE_RANGE, render_node},
        tool_error,
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::Document;
use image::{ExtendedColorType, codecs::jpeg::JpegEncoder};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ResourceContents},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::{borrow::Cow, collections::HashSet};

mod originals;

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
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AssetFormat {
    /// Lossless, keeps transparency.
    #[default]
    Png,
    /// Lossy; transparent areas are flattened onto white.
    Jpg,
}

/// Inputs for asset download.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssetsInput {
    /// 1 to 20 node IDs or Figma URLs with `node-id`; instance paths (I1:2;3:4) are accepted.
    node_ids: Vec<String>,
    /// Registered file stem or full path; optional for a single file.
    file_key: Option<String>,
    /// `png` (default) or `jpg`.
    format: Option<AssetFormat>,
    /// Pixels per design unit, 0.01 to 4 (default 1). Without it the longest side is capped at
    /// 2048 px; with it, at 4096 px.
    scale: Option<f64>,
    /// Return the original bytes of the image fills inside each node instead of a render.
    original: Option<bool>,
}

#[tool_router(router = assets_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Download up to 20 nodes as PNG or JPG images (format, scale 0.01 to 4; longest side capped at 2048 px without scale, 4096 px with it). JPG has no transparency: nodes are flattened onto white, pages keep their page background. With original=true the call returns the source files of the image fills inside each node (up to 12 per call, each once) with their detected MIME type instead of a render. Each node gets a text line; a node that fails is reported there and does not fail the call. Instance paths (I1:2;3:4) are accepted. SVG and PDF export are not available offline."
    )]
    async fn download_assets(&self, Parameters(input): Parameters<AssetsInput>) -> CallToolResult {
        let document = match self.files().open(input.file_key.as_deref()).await {
            Ok(document) => document,
            Err(error) => return tool_error(error),
        };
        let rendering = match self
            .files()
            .rendering(input.file_key.as_deref(), !self.no_system_fonts())
        {
            Ok(rendering) => rendering,
            Err(error) => return tool_error(error),
        };
        match tokio::task::spawn_blocking(move || {
            download(
                &document,
                &input.node_ids,
                input.format.unwrap_or_default(),
                input.scale,
                input.original.unwrap_or(false),
                &rendering,
            )
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::load_task(error)),
        }
    }
}

fn failure(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
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
) -> CallToolResult {
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
) -> CallToolResult {
    if node_ids.is_empty() || node_ids.len() > MAX_NODES {
        return failure(format!("nodeIds must hold 1 to {MAX_NODES} ids"));
    }
    if scale.is_some_and(|s| !SCALE_RANGE.contains(&s)) {
        return failure("scale must be between 0.01 and 4");
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
            Err(message) => {
                failed += 1;
                content.push(ContentBlock::text(format!("{id}: error: {message}")));
            }
        }
    }
    let all_failed = failed == node_ids.len();
    let content = text_first(content);
    if all_failed {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

/// The same items with every text block before every image or blob, so the summary and
/// omitted lines reach the agent even when the reply is cut after the images.
pub(crate) fn text_first(content: Vec<ContentBlock>) -> Vec<ContentBlock> {
    let (mut text, media): (Vec<_>, Vec<_>) =
        content.into_iter().partition(|c| c.as_text().is_some());
    text.extend(media);
    text
}

/// Raw bytes (before base64) still allowed in one reply.
#[derive(Debug)]
pub(crate) struct Payload(usize);

impl Payload {
    pub(crate) fn new(max: usize) -> Self {
        Self(max)
    }

    /// Whether anything more fits at all.
    pub(crate) fn is_full(&self) -> bool {
        self.0 == 0
    }

    /// Whether `bytes` more would fit.
    pub(crate) fn fits(&self, bytes: usize) -> bool {
        bytes <= self.0
    }

    /// Reserves `bytes`; `false` (and nothing taken) when they do not fit.
    pub(crate) fn take(&mut self, bytes: usize) -> bool {
        match self.0.checked_sub(bytes) {
            Some(rest) => {
                self.0 = rest;
                true
            }
            None => false,
        }
    }

    /// The text of an item that did not fit the reply's budget.
    pub(crate) fn omitted(id: &str) -> String {
        format!("{id}: omitted, the reply holds at most {PAYLOAD_LABEL} of image data")
    }

    /// The text of an image over [`MAX_IMAGE_BYTES`].
    pub(crate) fn too_large(id: &str, bytes: usize) -> String {
        format!(
            "{id}: omitted, image of {bytes} bytes is over the {IMAGE_CAP_LABEL} per-image limit"
        )
    }
}

/// What one call may still return: image bytes, original images and which were sent.
struct Budget {
    payload: Payload,
    originals: usize,
    sent: HashSet<String>,
}

impl Budget {
    fn new(max_payload: usize) -> Self {
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
) -> Result<Vec<ContentBlock>, String> {
    if payload.is_full() {
        return Err(Payload::omitted(id));
    }
    let shot = render_node(doc, id, scale, rendering)?;
    let mut text = format!("{id}: {}", shot.describe());
    let pixmap = shot.frame.pixmap;
    let (bytes, mime) = match format {
        AssetFormat::Png => (
            pixmap
                .into_png()
                .map_err(|e| format!("PNG encoding failed: {e}"))?,
            "image/png",
        ),
        AssetFormat::Jpg => {
            text.push_str("\nflattened onto white");
            let (w, h) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
            (jpeg(pixmap.data_as_u8_slice(), w, h)?, "image/jpeg")
        }
    };
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(Payload::too_large(id, bytes.len()));
    }
    if !payload.take(bytes.len()) {
        return Err(Payload::omitted(id));
    }
    Ok(vec![
        ContentBlock::text(text),
        ContentBlock::image(STANDARD.encode(bytes), mime),
    ])
}

/// JPEG of premultiplied RGBA8 `pixels` composited over white.
fn jpeg(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
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
        .map_err(|e| format!("JPEG encoding failed: {e}"))?;
    Ok(out)
}

/// The original files behind the image fills of `id` that this call has not sent yet, as
/// image or blob items. Fails when the node has no image fills or none has stored bytes.
/// Images of a lazily opened document are read from the file only once they fit the budget.
fn original_images(
    doc: &Document,
    id: &str,
    rendering: &Rendering,
    budget: &mut Budget,
) -> Result<Vec<ContentBlock>, String> {
    let hashes = originals::image_hashes(doc, id)?;
    if hashes.is_empty() {
        return Err("no image fills inside this node".into());
    }
    let mut content = vec![];
    let (mut missing, mut omitted, mut repeated) = (0, 0, 0);
    let mut too_large = Vec::new();
    for hash in &hashes {
        if budget.sent.contains(hash) {
            repeated += 1;
            continue;
        }
        let bytes = if let Some(bytes) = doc.images.get(hash) {
            Cow::Borrowed(bytes.as_slice())
        } else if let Some(entry) = doc.lazy_images.get(hash) {
            // The directory's size, checked before reading so a huge image is never loaded.
            let size = usize::try_from(entry.size).unwrap_or(usize::MAX);
            if size > MAX_IMAGE_BYTES {
                too_large.push(format!(
                    "{id}: image {hash} omitted, {size} bytes is over the {IMAGE_CAP_LABEL} per-image limit"
                ));
                continue;
            }
            if budget.originals == 0 || !budget.payload.fits(size) {
                omitted += 1;
                continue;
            }
            match rendering.read(entry) {
                Some(bytes) => Cow::Owned(bytes),
                None => {
                    missing += 1;
                    continue;
                }
            }
        } else {
            missing += 1;
            continue;
        };
        if bytes.len() > MAX_IMAGE_BYTES {
            too_large.push(format!(
                "{id}: image {hash} omitted, {} bytes is over the {IMAGE_CAP_LABEL} per-image limit",
                bytes.len()
            ));
            continue;
        }
        if budget.originals == 0 || !budget.payload.take(bytes.len()) {
            omitted += 1;
            continue;
        }
        budget.originals -= 1;
        budget.sent.insert(hash.clone());
        let data = STANDARD.encode(&bytes);
        let mime = image::guess_format(&bytes).ok().map(|f| f.to_mime_type());
        content.push(match mime {
            Some(mime) => ContentBlock::image(data, mime),
            None => ContentBlock::resource(
                ResourceContents::blob(data, format!("figma-image://{hash}"))
                    .with_mime_type("application/octet-stream"),
            ),
        });
        content.push(ContentBlock::text(format!(
            "{id}: image {hash} ({} bytes, {})",
            bytes.len(),
            mime.unwrap_or("unknown type")
        )));
    }
    if missing == hashes.len() {
        return Err(format!("{missing} image(s) not stored in the file"));
    }
    for line in too_large {
        content.push(ContentBlock::text(line));
    }
    if missing + omitted + repeated > 0 {
        content.push(ContentBlock::text(format!(
            "{id}: {missing} image(s) not stored in the file, {omitted} omitted (limit {MAX_ORIGINALS} images and {PAYLOAD_LABEL} per call), {repeated} already returned for another node"
        )));
    }
    Ok(content)
}
