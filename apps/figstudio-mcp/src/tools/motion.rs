//! `get_motion_context`: keyframe animation with CSS and motion.dev code.
use crate::{
    FigstudioServer,
    tools::{to_call, tool_error},
};
use figstudio_mcp_core::motion;
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

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
}
