//! `download_assets`: PNG or JPEG renders of nodes, or the original bytes of their images.
use crate::{
    FigstudioServer,
    tools::{
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

mod originals;

/// Most nodes one call may name.
pub const MAX_NODES: usize = 20;
/// Most original images one call returns, so a page of photos cannot flood the reply.
pub const MAX_ORIGINALS: usize = 20;
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
    /// Pixels per design unit, 0.01 to 4 (default 1). The longest side is capped at 4096 px.
    scale: Option<f64>,
    /// Return the original bytes of the image fills inside each node instead of a render.
    original: Option<bool>,
}

#[tool_router(router = assets_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Download up to 20 nodes as PNG or JPG images (format, scale 0.01 to 4, longest side capped at 4096 px). JPG has no transparency: nodes are flattened onto white, pages keep their page background. With original=true the call returns the source files of the image fills inside each node (up to 20 per call, each once) with their detected MIME type instead of a render. Each node gets a text line; a node that fails is reported there and does not fail the call. Instance paths (I1:2;3:4) are accepted. SVG and PDF export are not available offline."
    )]
    async fn download_assets(&self, Parameters(input): Parameters<AssetsInput>) -> CallToolResult {
        let document = match self.files().open(input.file_key.as_deref()).await {
            Ok(document) => document,
            Err(error) => return tool_error(error),
        };
        let system_fonts = !self.no_system_fonts();
        match tokio::task::spawn_blocking(move || {
            download(
                &document,
                &input.node_ids,
                input.format.unwrap_or_default(),
                input.scale,
                input.original.unwrap_or(false),
                system_fonts,
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
    system_fonts: bool,
) -> CallToolResult {
    if node_ids.is_empty() || node_ids.len() > MAX_NODES {
        return failure(format!("nodeIds must hold 1 to {MAX_NODES} ids"));
    }
    if scale.is_some_and(|s| !SCALE_RANGE.contains(&s)) {
        return failure("scale must be between 0.01 and 4");
    }
    let mut content = Vec::new();
    let mut failed = 0;
    let mut budget = MAX_ORIGINALS;
    for id in node_ids {
        let outcome = if original {
            original_images(doc, id, &mut budget)
        } else {
            rendered(doc, id, format, scale, system_fonts)
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
    if all_failed {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

/// A text line and the encoded image of one node.
fn rendered(
    doc: &Document,
    id: &str,
    format: AssetFormat,
    scale: Option<f64>,
    system_fonts: bool,
) -> Result<Vec<ContentBlock>, String> {
    let shot = render_node(doc, id, scale, system_fonts)?;
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

/// The original files behind the image fills of `id`, as image or blob items.
fn original_images(
    doc: &Document,
    id: &str,
    budget: &mut usize,
) -> Result<Vec<ContentBlock>, String> {
    let hashes = originals::image_hashes(doc, id)?;
    let mut content = vec![];
    let (mut missing, mut skipped) = (0, 0);
    for hash in &hashes {
        let Some(bytes) = doc.images.get(hash) else {
            missing += 1;
            continue;
        };
        if *budget == 0 {
            skipped += 1;
            continue;
        }
        *budget -= 1;
        let data = STANDARD.encode(bytes);
        let mime = image::guess_format(bytes).ok().map(|f| f.to_mime_type());
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
    if hashes.is_empty() {
        return Err("no image fills inside this node".into());
    }
    if missing + skipped > 0 {
        content.push(ContentBlock::text(format!(
            "{id}: {missing} image(s) not stored in the file, {skipped} omitted (limit {MAX_ORIGINALS} per call)"
        )));
    }
    Ok(content)
}
