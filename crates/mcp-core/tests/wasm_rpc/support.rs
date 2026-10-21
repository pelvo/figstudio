//! Public-fixture setup and wire-level assertions.
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::{Document, Guid};
use figstudio_mcp_core::{
    CoreError,
    rendering::Rendering,
    wasm_rpc::{McpTab, McpTabEnv, handle_request},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub struct Env {
    pub tabs: Vec<McpTab>,
    pub revealed: Mutex<Vec<(u64, Guid)>>,
}
impl McpTabEnv for Env {
    fn tabs(&self) -> &[McpTab] {
        &self.tabs
    }
    fn active_tab(&self) -> Option<u64> {
        Some(7)
    }
    fn reveal(&self, tab: u64, node: Guid) -> Result<(), CoreError> {
        self.revealed.lock().unwrap().push((tab, node));
        Ok(())
    }
}
pub fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
}
pub fn env() -> Env {
    let doc = Arc::new(Document::open(fixture()).unwrap());
    let page = doc.nodes.iter().find(|n| n.kind == "CANVAS").unwrap().id;
    Env {
        tabs: vec![McpTab {
            id: 7,
            name: "structure.fig".into(),
            file_key: "structure".into(),
            doc,
            rendering: Rendering::default(),
            page,
            selection: Some(Guid(1, 34)),
            viewport: [0., 0., 800., 600.],
        }],
        revealed: Mutex::default(),
    }
}
pub fn rpc(env: &Env, method: &str, params: Value) -> Value {
    serde_json::from_str(
        &handle_request(
            &json!({"jsonrpc":"2.0","id":"request","method":method,"params":params}).to_string(),
            env,
        )
        .unwrap(),
    )
    .unwrap()
}
pub fn call(env: &Env, name: &str, args: Value) -> Value {
    let reply = rpc(env, "tools/call", json!({"name":name,"arguments":args}));
    assert!(reply.get("error").is_none(), "{reply}");
    reply["result"].clone()
}
pub fn text(value: &Value) -> String {
    value["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn pixels(value: &Value, mime: &str, format: image::ImageFormat) -> image::RgbaImage {
    let image = value["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["type"] == "image")
        .unwrap();
    assert_eq!(image["mimeType"], mime);
    let bytes = STANDARD.decode(image["data"].as_str().unwrap()).unwrap();
    assert_eq!(image::guess_format(&bytes).unwrap(), format);
    let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
    assert!(decoded.width() > 0 && decoded.height() > 0);
    decoded
}
