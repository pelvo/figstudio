//! Shader fills and effects a file uses; their source stays in Figma's cloud.
use super::{to_call, tool_error};
use crate::FigstudioServer;
use figstudio_mcp_core::shaders;
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

/// Inputs for `list_file_shaders`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListInput {
    /// Registered file stem or full path; optional for a single file.
    file_key: Option<String>,
}

/// Inputs for `get_shader`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetInput {
    /// The shader ID from `list_file_shaders`, its key, or its node ID.
    id: String,
    /// Registered file stem or full path; optional for a single file.
    file_key: Option<String>,
}

#[tool_router(router = shaders_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "List the shader fills and effects this file uses (at most 100): id, nodeId, key, type (fill or effect), version, published, parameter names and how many layers use each. Offline, name, description and source files are null or empty: the .fig does not hold them, they stay in Figma's cloud. Use get_shader for parameters and the layers using a shader."
    )]
    async fn list_file_shaders(&self, Parameters(input): Parameters<ListInput>) -> CallToolResult {
        let doc = match self.files().open(input.file_key.as_deref()).await {
            Ok(doc) => doc,
            Err(error) => return tool_error(error),
        };
        match tokio::task::spawn_blocking(move || to_call(shaders::list(&doc))).await {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::token_task(error)),
        }
    }

    #[tool(
        description = "Read one shader this file uses, by the id from list_file_shaders (or its key or node ID): its parameters (name, label, type, control, default, min/max/step or select options) and the layers that use it (at most 200) with the field holding it and the values they set. No source code offline: the .fig does not hold it."
    )]
    async fn get_shader(&self, Parameters(input): Parameters<GetInput>) -> CallToolResult {
        let doc = match self.files().open(input.file_key.as_deref()).await {
            Ok(doc) => doc,
            Err(error) => return tool_error(error),
        };
        match tokio::task::spawn_blocking(move || to_call(shaders::get(&doc, &input.id))).await {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::token_task(error)),
        }
    }
}
