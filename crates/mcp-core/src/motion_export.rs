//! Exports a layer's animation as a file (GIF, PNG frames, WebM or MP4): every frame of one
//! pass of its timeline, drawn as the motion preview draws it (the page region under the
//! timeline's root, with the page behind it), then encoded by `figstudio-video`.

use std::sync::Arc;

use figstudio_fig::{Document, Guid};
use figstudio_render::Viewport;
use figstudio_render::kurbo::{Affine, Rect};
use figstudio_render::motion::Playhead;
use figstudio_video::{Format, frame_size};

use crate::error::CoreError;
use crate::motion::{Player, has_motion, node_motion};
use crate::node_id::{NodeId, parse_node_id};
use crate::rendering::Rendering;
use crate::screenshot::Root;

/// Longest export, in seconds of animation.
pub const MAX_SECONDS: f64 = 60.0;

/// Frames a second when none is asked for: 30, or 25 for GIF (exact in its hundredths).
pub fn default_fps(format: Format) -> u32 {
    if format == Format::Gif { 25 } else { 30 }
}

/// What an export draws: the region, its size in pixels, and how many frames at what rate.
#[derive(Debug)]
pub struct Plan {
    /// The timeline's root (the layer whose region is drawn).
    pub root: Guid,
    page: Guid,
    region: Rect,
    scale: f64,
    pub format: Format,
    pub width: u16,
    pub height: u16,
    pub fps: u32,
    pub frames: u32,
    /// Length of one pass, µs (a boomerang plays there and back).
    pub period_us: f64,
    pub mode: &'static str,
    /// The pass was longer than [`MAX_SECONDS`] and was cut.
    pub cut: bool,
    player: Arc<Player>,
}

/// Plans exporting the animation `node_id` plays in as `format`, at `fps` frames a second
/// (default [`default_fps`]) and `scale` pixels per layer unit (default 1, lowered to fit
/// `figstudio_video::MAX_SIDE`).
pub fn plan(
    doc: &Document,
    node_id: &str,
    format: Format,
    fps: Option<u32>,
    scale: Option<f64>,
) -> Result<Plan, CoreError> {
    if !format.available() {
        return Err(CoreError::input(format!(
            "{} is not available in this build",
            format.label()
        )));
    }
    let id = match parse_node_id(node_id)? {
        NodeId::Node(id) => id,
        NodeId::InstancePath(_) => {
            return Err(CoreError::input(
                "export the instance itself, not a layer inside it",
            ));
        }
    };
    let node = doc
        .node(id)
        .ok_or_else(|| CoreError::input(format!("node {id} does not exist in this file")))?;
    // The whole animation: the timeline's root when the layer is part of one.
    let root = node_motion(doc, &id.to_string())
        .and_then(|m| m["timelineRootId"].as_str().map(str::to_owned))
        .and_then(|r| match parse_node_id(&r) {
            Ok(NodeId::Node(g)) => Some(g),
            _ => None,
        })
        .unwrap_or(id);
    let player = Arc::new(Player::new(doc));
    let (duration_us, mode) = player.timeline(doc, root);
    if duration_us <= 0.0 || (root == id && !has_motion(&node.data) && doc.children(id).is_empty())
    {
        return Err(CoreError::input(format!("node {id} has no animation")));
    }
    let page = doc
        .page_of(root)
        .ok_or_else(|| CoreError::input(format!("node {root} is not on a page")))?;
    let region = figstudio_render::node_bounds(doc, root)
        .filter(|b| b.width() > 0.0 && b.height() > 0.0)
        .ok_or_else(|| CoreError::render(format!("node {root} has nothing to draw")))?;
    let fps = fps.unwrap_or_else(|| default_fps(format));
    if !figstudio_video::FPS_RANGE.contains(&fps) {
        return Err(CoreError::input(format!(
            "fps must be between {} and {}",
            figstudio_video::FPS_RANGE.start(),
            figstudio_video::FPS_RANGE.end()
        )));
    }
    let wanted = scale.unwrap_or(1.0);
    if !(0.01..=8.0).contains(&wanted) {
        return Err(CoreError::input("scale must be between 0.01 and 8"));
    }
    let longest = region.width().max(region.height());
    let scale = wanted.min(f64::from(figstudio_video::MAX_SIDE) / longest);
    let (width, height) = frame_size(
        format,
        (region.width() * scale).round() as u32,
        (region.height() * scale).round() as u32,
    );
    let period_us = if mode == "boomerang" {
        2.0 * duration_us
    } else {
        duration_us
    };
    let seconds = (period_us / 1e6).min(MAX_SECONDS);
    let frames = ((seconds * f64::from(fps)).round() as u32).max(1);
    Ok(Plan {
        root,
        page,
        region,
        scale,
        format,
        width,
        height,
        fps,
        frames,
        period_us,
        mode,
        cut: period_us / 1e6 > MAX_SECONDS,
        player,
    })
}

/// Draws every frame of `plan` and gives each to `frame` (RGBA8, opaque: the page is behind).
/// Returns the first frame's render stats (what it did not draw exactly).
pub fn render<E>(
    doc: &Document,
    plan: &Plan,
    rendering: &Rendering,
    mut frame: impl FnMut(&[u8]) -> Result<(), E>,
) -> Result<figstudio_render::Stats, E> {
    let background = Some(figstudio_render::page_background(doc, plan.page));
    let viewport = Viewport {
        width: plan.width,
        height: plan.height,
        transform: Affine::scale(plan.scale)
            * Affine::translate((-plan.region.x0, -plan.region.y0)),
        background,
    };
    let mut stats = None;
    for i in 0..plan.frames {
        let playhead = Playhead {
            motion: plan.player.clone(),
            time_us: f64::from(i) * 1e6 / f64::from(plan.fps),
        };
        let drawn = rendering.draw_at(doc, Root::Node(plan.page), &viewport, Some(playhead));
        frame(drawn.pixmap.data_as_u8_slice())?;
        stats.get_or_insert(drawn.stats);
    }
    Ok(stats.unwrap_or_default())
}

/// A finished export.
#[derive(Debug)]
pub struct Exported {
    pub bytes: Vec<u8>,
    pub plan: Plan,
    /// The first frame's render stats.
    pub stats: figstudio_render::Stats,
}

/// [`plan`], [`render`] and the encoder.
pub fn export(
    doc: &Document,
    node_id: &str,
    format: Format,
    fps: Option<u32>,
    scale: Option<f64>,
    rendering: &Rendering,
) -> Result<Exported, CoreError> {
    let plan = plan(doc, node_id, format, fps, scale)?;
    let mut encoder = figstudio_video::encoder(format, plan.width, plan.height, plan.fps)
        .map_err(|e| CoreError::encode(format.label(), e))?;
    let stats = render(doc, &plan, rendering, |rgba| encoder.push(rgba))
        .map_err(|e| CoreError::encode(format.label(), e))?;
    let bytes = encoder
        .finish()
        .map_err(|e| CoreError::encode(format.label(), e))?;
    Ok(Exported { bytes, plan, stats })
}

/// `export_motion`: the animation of `node_id` in file `key` as `format`. With an assets
/// folder the file is written there and the reply names it; otherwise it comes back inline
/// (a GIF as an image, the others as a resource blob), up to
/// [`crate::assets::MAX_IMAGE_BYTES`].
#[expect(
    clippy::too_many_arguments,
    reason = "the tool's inputs and the server's settings"
)]
pub fn tool(
    doc: &Document,
    key: &str,
    node_id: &str,
    format: Format,
    fps: Option<u32>,
    scale: Option<f64>,
    rendering: &Rendering,
    assets: &crate::design::Assets,
) -> crate::output::Output {
    use base64::{Engine, engine::general_purpose::STANDARD};

    use crate::output::{Block, Output};

    let Exported { bytes, plan, stats } = match export(doc, node_id, format, fps, scale, rendering)
    {
        Ok(done) => done,
        Err(e) => return Output::fault(e.to_string()),
    };
    let notes = crate::screenshot::approximations(&stats);
    let mut line = format!(
        "{}x{}, {} frames at {} fps ({:.2} s of a {} timeline), {}, {} bytes",
        plan.width,
        plan.height,
        plan.frames,
        plan.fps,
        f64::from(plan.frames) / f64::from(plan.fps),
        plan.mode,
        format.label(),
        bytes.len()
    );
    if plan.cut {
        line.push_str(&format!(", cut to {MAX_SECONDS} s"));
    }
    if !notes.is_empty() {
        line.push_str(&format!("\napproximations: {}", notes.join(", ")));
    }
    let name = crate::design::assets::file_name(key, &plan.root.to_string(), format.extension());
    match assets {
        #[cfg(feature = "fs")]
        crate::design::Assets::Dir(dir) => match crate::design::assets::write(dir, &name, &bytes) {
            Ok(path) => Output::success(vec![Block::text(format!("{}: {line}", path.display()))]),
            Err(e) => Output::fault(format!("cannot write {name}: {e}")),
        },
        _ => {
            if bytes.len() > crate::assets::MAX_IMAGE_BYTES {
                return Output::fault(format!(
                    "{line}: over {} inline; lower scale or fps, or start the server with an \
                     assets folder",
                    crate::assets::IMAGE_CAP_LABEL
                ));
            }
            let data = STANDARD.encode(&bytes);
            let block = if format == Format::Gif {
                Block::image(data, format.mime())
            } else {
                Block::Blob {
                    data_b64: data,
                    uri: format!("figstudio://motion/{name}"),
                    mime: format.mime().to_owned(),
                }
            };
            Output {
                content: vec![Block::text(format!("{name}: {line}")), block],
                structured: None,
                is_error: false,
            }
        }
    }
}
