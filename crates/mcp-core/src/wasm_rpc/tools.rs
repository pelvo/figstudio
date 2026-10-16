use super::{McpTabEnv, live};
use crate::{assets, design, output::Output};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Format {
    Png,
    Jpg,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    node_id: Option<String>,
    file_key: Option<String>,
    depth: Option<u32>,
    scale: Option<f64>,
    region: Option<String>,
    page: Option<String>,
    rect: Option<[f64; 4]>,
    limit: Option<usize>,
    node_ids: Option<Vec<String>>,
    format: Option<Format>,
    original: Option<bool>,
    include_images: Option<bool>,
    image_scale: Option<f64>,
    client_frameworks: Option<String>,
    client_languages: Option<String>,
    max_nodes: Option<usize>,
    query: Option<String>,
    kinds: Option<Vec<String>>,
}

pub(super) fn call(params: &Value, env: &dyn McpTabEnv) -> Result<Output, String> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or("Tool name is required")?;
    if !super::tool_list().iter().any(|t| t["name"] == name) {
        return Err(format!("Unknown tool {name}"));
    }
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !args.is_object() {
        return Err("Tool arguments must be an object".into());
    }
    let input: Input = serde_json::from_value(args).map_err(|e| e.to_string())?;
    if (name == "reveal_node" && input.node_id.is_none())
        || (name == "search_design_system" && input.query.is_none())
    {
        return Err("Missing required tool argument".into());
    }
    if name == "list_open_tabs" {
        return Ok(live::tabs(env));
    }
    if name == "list_files" {
        return Ok(Output::structured(
            json!({"files":env.tabs().iter().map(|t| json!({"fileKey":t.file_key,"name":t.name})).collect::<Vec<_>>()}),
        ));
    }
    let link = input
        .node_id
        .as_deref()
        .or(input.region.as_deref())
        .or_else(|| input.node_ids.as_ref()?.first().map(String::as_str));
    let tab = match live::pick(env, input.file_key.as_deref(), link) {
        Ok(tab) => tab,
        Err(e) => return Ok(Output::fault(e.to_string())),
    };
    let doc = &tab.doc;
    let node = input.node_id.as_deref();
    let target = || live::target(tab, node);
    let result = (|| {
        Ok(match name {
            "get_selection" => live::selection(tab),
            "reveal_node" => live::reveal(env, tab, &target()?),
            "get_metadata" => crate::metadata::metadata(doc, node, input.depth),
            "get_screenshot" => {
                crate::screenshot::screenshot(doc, &target()?, input.scale, &tab.rendering)
            }
            "get_nodes_in_region" => crate::region_tool::nodes_in_region(
                doc,
                input.region.as_deref(),
                input.page.as_deref(),
                input.rect,
                input.limit,
            ),
            "get_variable_defs" => crate::variables::variable_defs(doc, &target()?),
            "get_design_context" => crate::design::design_context(
                doc,
                &target()?,
                &design::Options {
                    file_key: tab.file_key.clone(),
                    frameworks: input.client_frameworks,
                    languages: input.client_languages,
                    max_nodes: input.max_nodes,
                    assets: design::Assets::Inline,
                    rendering: tab.rendering.clone(),
                    ..design::Options::default()
                },
            ),
            "download_assets" => {
                let mut ids = input.node_ids.unwrap_or_default();
                if ids.is_empty() {
                    ids.push(target()?);
                }
                assets::download(
                    doc,
                    &ids,
                    match input.format {
                        Some(Format::Jpg) => assets::AssetFormat::Jpg,
                        _ => assets::AssetFormat::Png,
                    },
                    input.scale,
                    input.original.unwrap_or(false),
                    &tab.rendering,
                )
            }
            "get_figjam" => crate::figjam::figjam(
                doc,
                node,
                input.depth,
                input.include_images.unwrap_or(false),
                input.image_scale,
                &tab.rendering,
            ),
            "search_design_system" => crate::design_system::search(
                doc,
                input.query.as_deref().unwrap_or_default(),
                input.kinds.as_deref(),
            ),
            "get_libraries" => Output::structured(crate::design_system::summary(doc)),
            "get_context_for_code_connect" => {
                crate::codeconnect_context::component_context(doc, &target()?)
            }
            _ => Output::fault("Tool is unavailable"),
        })
    })();
    Ok(result.unwrap_or_else(|e: crate::CoreError| Output::fault(e.to_string())))
}

pub(super) fn prompt(params: &Value, env: &dyn McpTabEnv) -> Result<Value, String> {
    if params.get("name").and_then(Value::as_str) != Some("create_design_system_rules") {
        return Err("Unknown prompt".into());
    }
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let args: std::collections::BTreeMap<String, String> =
        serde_json::from_value(args).map_err(|e| e.to_string())?;
    let tab = live::pick(env, args.get("fileKey").map(String::as_str), None)
        .map_err(|e| e.to_string())?;
    let text = crate::prompts::rules_text(
        &tab.doc,
        args.get("clientFrameworks").map(String::as_str),
        args.get("clientLanguages").map(String::as_str),
    );
    Ok(
        json!({"description":crate::prompts::RULES_DESCRIPTION,"messages":[{"role":"user","content":{"type":"text","text":text}}]}),
    )
}
