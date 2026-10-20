//! Local Code Connect context and sidecar mappings; never changes the document.
mod sidecar;

use super::{to_call, tool_error};
use crate::FigstudioServer;
use figstudio_mcp_core::codeconnect_context;
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Return schema-preserving component properties and bounded local child context.
pub fn component_context(doc: &figstudio_fig::Document, input: &str) -> CallToolResult {
    to_call(codeconnect_context::component_context(doc, input))
}

/// A local source binding for a component.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    pub component_name: String,
    pub source: String,
    pub label: String,
    pub snippet: Option<String>,
    #[serde(default, alias = "imports")]
    pub snippet_imports: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextInput {
    /// Node ID, instance path or Figma URL. Required, except in the app's live mode, where it
    /// defaults to the selection.
    node_id: Option<String>,
    file_key: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MapInput {
    node_id: Option<String>,
    file_key: Option<String>,
    client_frameworks: Option<Vec<String>>,
    client_languages: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddInput {
    node_id: String,
    file_key: Option<String>,
    component_name: String,
    source: String,
    label: String,
    snippet: Option<String>,
    #[serde(default, alias = "imports")]
    snippet_imports: Vec<String>,
}

#[tool_router(router = codeconnect_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Get local component properties, variants and nested instances. Accepts a component, component set or instance (including instance paths)."
    )]
    async fn get_context_for_code_connect(
        &self,
        Parameters(input): Parameters<ContextInput>,
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
        match tokio::task::spawn_blocking(move || component_context(&doc, &node_id)).await {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::token_task(error)),
        }
    }

    #[tool(
        description = "Read offline Code Connect mappings from FILE.fig.codeconnect.json. Lookup falls back from instance to component to component set; selected mappings use the requested node ID as their key. Framework filters match labels; language filters match source extensions or labels."
    )]
    async fn get_code_connect_map(
        &self,
        Parameters(input): Parameters<MapInput>,
    ) -> CallToolResult {
        self.codeconnect_map(input, None).await
    }

    #[tool(
        description = "Atomically add a local Code Connect mapping next to the registered .fig file, without changing the .fig. Instances map to their component. source is stored, never fetched or written. snippetImports accepts an array of imports (imports is a compatibility alias). Limits: source 4096 bytes, snippet 100000 bytes, 100 imports of 4096 bytes each."
    )]
    async fn add_code_connect_map(
        &self,
        Parameters(input): Parameters<AddInput>,
    ) -> CallToolResult {
        let mapping = Mapping {
            component_name: input.component_name,
            source: input.source,
            label: input.label,
            snippet: input.snippet,
            snippet_imports: input.snippet_imports,
        };
        self.codeconnect_map(
            MapInput {
                node_id: Some(input.node_id),
                file_key: input.file_key,
                client_frameworks: None,
                client_languages: None,
            },
            Some(mapping),
        )
        .await
    }

    async fn codeconnect_map(&self, input: MapInput, add: Option<Mapping>) -> CallToolResult {
        let key = match self.file_for(input.file_key.as_deref(), input.node_id.as_deref()) {
            Ok(key) => key,
            Err(error) => return tool_error(error),
        };
        let (doc, _, path) = match self.files().open_picked(key.as_deref()).await {
            Ok(picked) => picked,
            Err(error) => return tool_error(error),
        };
        let lock = self.codeconnect_lock.clone();
        match tokio::task::spawn_blocking(move || {
            let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            sidecar::map(&doc, &path, input, add)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::token_task(error)),
        }
    }
}

type Mappings = BTreeMap<String, Mapping>;

fn fault(message: &str) -> CallToolResult {
    CallToolResult::error(vec![rmcp::model::ContentBlock::text(message)])
}
