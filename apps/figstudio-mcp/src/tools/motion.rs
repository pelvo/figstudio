//! `get_motion_context`: keyframe animation with CSS and motion.dev code; `export_motion`: the
//! animation as a video, GIF or PNG frames.
use crate::{
    FigstudioServer,
    tools::{to_call, tool_error},
};
use figstudio_mcp_core::motion;
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

/// File format of `export_motion`.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MotionFormat {
    /// MP4: H.264 from the OS (macOS, Windows), else AV1.
    #[default]
    Mp4,
    /// WebM with AV1.
    Webm,
    /// Animated GIF, looping (256 colours a frame).
    Gif,
    /// A zip of numbered PNG frames, lossless.
    Frames,
}

impl From<MotionFormat> for figstudio_video::Format {
    fn from(format: MotionFormat) -> Self {
        match format {
            MotionFormat::Mp4 => Self::Mp4,
            MotionFormat::Webm => Self::Webm,
            MotionFormat::Gif => Self::Gif,
            MotionFormat::Frames => Self::Frames,
        }
    }
}

/// Inputs for `export_motion`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportMotionInput {
    /// Node ID or Figma URL of an animated layer (its timeline's root is drawn). Required,
    /// except in the app's live mode, where it defaults to the selection.
    node_id: Option<String>,
    /// Registered file key; optional when just one file is registered.
    file_key: Option<String>,
    /// `mp4` (default), `webm`, `gif` or `frames` (a zip of PNGs).
    format: Option<MotionFormat>,
    /// Frames a second, 1 to 60 (default 30, 25 for gif).
    fps: Option<u32>,
    /// Pixels per design unit, 0.01 to 8 (default 1; the longest side stays within 3840 px).
    scale: Option<f64>,
}

/// Inputs for `get_motion_context`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MotionInput {
    /// Node ID, instance path (I1:2;3:4) or Figma URL. Required, except in the app's live mode,
    /// where it defaults to the selection.
    node_id: Option<String>,
    /// Registered file key; optional when just one file is registered.
    file_key: Option<String>,
    /// Also return every animated descendant, instance content included (at most 500).
    recursive: Option<bool>,
}

#[tool_router(router = motion_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Get the keyframe animation of a node so it can be rebuilt in code: per animated node, codeSnippets (CSS @keyframes, one animation per property over the whole timeline, and a motion.dev animate call) and keyframeBindings (field, operation, preset, delay, keyframes with time in ms, value and easing). recursive: true adds every animated descendant, instance content included (at most 500), and timelineCohorts groups nodes sharing one timeline with durationMs and loopMode. Use after get_design_context with the same nodeId. Output stops at the server's --max-output-bytes (truncated: true)."
    )]
    async fn get_motion_context(
        &self,
        Parameters(input): Parameters<MotionInput>,
    ) -> CallToolResult {
        let key = match self.file_for(input.file_key.as_deref(), input.node_id.as_deref()) {
            Ok(key) => key,
            Err(error) => return tool_error(error),
        };
        let (key, node_id) = match self.target(key, input.node_id) {
            Ok(target) => target,
            Err(error) => return tool_error(error),
        };
        let doc = match self.files().open(key.as_deref()).await {
            Ok(doc) => doc,
            Err(error) => return tool_error(error),
        };
        let recursive = input.recursive.unwrap_or(false);
        let max_bytes = self.max_output_bytes();
        match tokio::task::spawn_blocking(move || {
            to_call(motion::motion_context(&doc, &node_id, recursive, max_bytes))
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::token_task(error)),
        }
    }

    #[tool(
        description = "Export the keyframe animation of a layer as a file: one pass of its timeline (a boomerang there and back), every frame drawn locally as in the app's motion preview (the timeline's root with the page behind it). format: mp4 (default; H.264 from the OS on macOS and Windows, else AV1), webm (AV1), gif (looping, 256 colours) or frames (a zip of PNGs). fps 1-60 (default 30, gif 25), scale 0.01-8 (default 1, longest side at most 3840 px), at most 60 s. With the server's assets directory the file is written there and its path returned; otherwise it is returned inline (gif as an image, others as a resource), up to 3.75 MB. Use get_motion_context for the keyframes themselves."
    )]
    async fn export_motion(
        &self,
        Parameters(input): Parameters<ExportMotionInput>,
    ) -> CallToolResult {
        let key = match self.file_for(input.file_key.as_deref(), input.node_id.as_deref()) {
            Ok(key) => key,
            Err(error) => return tool_error(error),
        };
        let (key, node_id) = match self.target(key, input.node_id) {
            Ok(target) => target,
            Err(error) => return tool_error(error),
        };
        let key = key.as_deref();
        let (doc, file_key, _) = match self.files().open_picked(key).await {
            Ok(picked) => picked,
            Err(error) => return tool_error(error),
        };
        let rendering = match self.files().rendering(key, &doc, !self.no_system_fonts()) {
            Ok(rendering) => rendering,
            Err(error) => return tool_error(error),
        };
        let assets = self.assets().clone();
        let format = input.format.unwrap_or_default().into();
        let (fps, scale) = (input.fps, input.scale);
        match tokio::task::spawn_blocking(move || {
            to_call(figstudio_mcp_core::motion_export::tool(
                &doc, &file_key, &node_id, format, fps, scale, &rendering, &assets,
            ))
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::token_task(error)),
        }
    }
}
