//! Offline design-system rule-file guidance through MCP prompts.
use crate::FigstudioServer;
use rmcp::{
    ErrorData,
    handler::server::wrapper::Parameters,
    model::{GetPromptResult, PromptMessage, Role},
    prompt, prompt_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RulesInput {
    /// Frameworks targeted by the rules, as a comma-separated string.
    client_frameworks: Option<String>,
    /// Languages targeted by the rules, as a comma-separated string.
    client_languages: Option<String>,
    /// Registered local file key; optional when only one file is loaded.
    file_key: Option<String>,
}

#[prompt_router(vis = "pub(crate)")]
impl FigstudioServer {
    #[prompt(
        description = "Create a design-system rules file from this local .fig file's components and tokens. Returns guidance only; does not write files or access team libraries."
    )]
    async fn create_design_system_rules(
        &self,
        Parameters(input): Parameters<RulesInput>,
    ) -> Result<GetPromptResult, ErrorData> {
        let doc = self
            .files()
            .open(input.file_key.as_deref())
            .await
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        let text = tokio::task::spawn_blocking(move || {
            figstudio_mcp_core::prompts::rules_text(
                &doc,
                input.client_frameworks.as_deref(),
                input.client_languages.as_deref(),
            )
        })
        .await
        .map_err(|_| ErrorData::internal_error("Design-system summary task failed", None))?;
        Ok(
            GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)])
                .with_description(figstudio_mcp_core::prompts::RULES_DESCRIPTION),
        )
    }
}
