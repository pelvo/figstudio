//! Browser RPC jobs: immutable snapshots cross the pool boundary, UI commands return by channel.
use crate::{mcp_state::viewport, mcp_wire::rpc_reply, tab::Tab};
use figstudio_fig::Guid;
use figstudio_mcp_core::{
    CoreError,
    rendering::Rendering,
    wasm_rpc::{McpTab, McpTabEnv, handle_request},
};
use std::sync::{
    Arc,
    mpsc::{Receiver, Sender, channel},
};

#[derive(Debug)]
pub(crate) enum Done {
    Reveal {
        generation: u32,
        tab: u64,
        node: Guid,
    },
    Reply {
        generation: u32,
        text: String,
    },
}

#[derive(Debug)]
pub(crate) struct Inbox {
    tx: Sender<Done>,
    pub rx: Receiver<Done>,
    pub pending: usize,
}

impl Default for Inbox {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, pending: 0 }
    }
}

#[derive(Debug)]
struct Snapshot {
    tabs: Vec<McpTab>,
    active: Option<u64>,
    tx: Sender<Done>,
    generation: u32,
}

impl McpTabEnv for Snapshot {
    fn tabs(&self) -> &[McpTab] {
        &self.tabs
    }
    fn active_tab(&self) -> Option<u64> {
        self.active
    }
    fn reveal(&self, tab: u64, node: Guid) -> Result<(), CoreError> {
        self.tx
            .send(Done::Reveal {
                generation: self.generation,
                tab,
                node,
            })
            .map_err(|_| CoreError::input("Browser UI disconnected"))
    }
}

impl Inbox {
    pub fn dispatch(
        &mut self,
        generation: u32,
        id: String,
        body: serde_json::Value,
        tabs: &[Tab],
        active: usize,
    ) {
        self.pending += 1;
        let keys = crate::mcp_wire::relay_keys(tabs);
        let env = Snapshot {
            tabs: tabs
                .iter()
                .zip(keys)
                .map(|(tab, file_key)| {
                    let source = tab.source.clone();
                    // Separate cache from the canvas worker's in-flight image requests.
                    let rendering = Rendering::with_reader(
                        Arc::new(move |entry| {
                            source.as_ref().and_then(|source| source.read(entry).ok())
                        }),
                        Default::default(),
                    );
                    McpTab {
                        id: tab.id,
                        name: tab.title(),
                        file_key,
                        doc: tab.doc.clone(),
                        rendering,
                        page: tab.page,
                        selection: tab.selection,
                        viewport: viewport(tab).unwrap_or([0.; 4]),
                    }
                })
                .collect(),
            active: tabs.get(active).map(|t| t.id),
            tx: self.tx.clone(),
            generation,
        };
        rayon::spawn(move || {
            let result = handle_request(&body.to_string(), &env);
            // The dispatcher returns serialized JSON; malformed output still completes the job.
            let text = rpc_reply(&id, result.as_deref()).unwrap_or_else(|_| {
                serde_json::json!({"type":"rpc_reply", "id":id, "status":500,
                    "body":{"jsonrpc":"2.0", "id":null,
                        "error":{"code":-32603,"message":"Could not encode MCP reply"}}})
                .to_string()
            });
            let _ = env.tx.send(Done::Reply { generation, text });
        });
    }
}
