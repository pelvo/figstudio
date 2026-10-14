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
    mpsc::{Receiver, Sender, TryRecvError, channel},
};
use std::time::Duration;
use web_time::Instant;

#[derive(Debug)]
pub(crate) enum Done {
    Reveal {
        generation: u32,
        tab: u64,
        node: Guid,
        /// The UI thread's answer; the worker waits on it with a bound.
        ack: Sender<Result<(), String>>,
    },
    Reply {
        generation: u32,
        text: String,
    },
}

/// How long a worker waits for the UI to apply a reveal before failing the call. Blocking
/// is safe here: this runs on the Rayon pool, never the UI thread, and the UI frame loop
/// keeps polling the inbox while replies are pending. The wait polls because std's
/// `Instant` (and so `recv_timeout`) is unsupported on wasm32; `web_time` and
/// `thread::sleep` work on the pool workers (as `worker.rs` already relies on).
const REVEAL_ACK_TIMEOUT: Duration = Duration::from_secs(2);
const ACK_POLL: Duration = Duration::from_millis(4);

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
    ack_timeout: Duration,
}

impl McpTabEnv for Snapshot {
    fn tabs(&self) -> &[McpTab] {
        &self.tabs
    }
    fn active_tab(&self) -> Option<u64> {
        self.active
    }
    fn reveal(&self, tab: u64, node: Guid) -> Result<(), CoreError> {
        let (ack, done) = channel();
        self.tx
            .send(Done::Reveal {
                generation: self.generation,
                tab,
                node,
                ack,
            })
            .map_err(|_| CoreError::input("Browser UI disconnected"))?;
        let deadline = Instant::now() + self.ack_timeout;
        loop {
            match done.try_recv() {
                Ok(result) => return result.map_err(CoreError::input),
                Err(TryRecvError::Disconnected) => {
                    return Err(CoreError::input("Browser UI disconnected"));
                }
                Err(TryRecvError::Empty) => {}
            }
            if Instant::now() >= deadline {
                return Err(CoreError::input(
                    "the browser tab did not apply the reveal within 2 s",
                ));
            }
            std::thread::sleep(ACK_POLL);
        }
    }
}

/// The UI thread's side of a reveal (run by `McpHost::frame`): apply it, or say why it
/// cannot run, so the worker's bounded wait always ends. `live` is the connection's
/// generation, or `None` when no relay connection is open.
pub(crate) fn apply_reveal(
    live: Option<u32>,
    generation: u32,
    tabs: &mut [Tab],
    active: &mut usize,
    tab: u64,
    node: Guid,
) -> Result<(), String> {
    match live {
        Some(current) if current == generation => crate::mcp_state::reveal(tabs, active, tab, node),
        Some(_) => Err("the tab reconnected; this reveal was dropped".to_owned()),
        None => Err("the browser tab disconnected".to_owned()),
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
            ack_timeout: REVEAL_ACK_TIMEOUT,
        };
        spawn(move || {
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

/// The browser runs RPC jobs on the Rayon pool; native builds compile this module for
/// tests only, where a plain thread stands in for the pool.
#[cfg(target_arch = "wasm32")]
fn spawn(f: impl FnOnce() + Send + 'static) {
    rayon::spawn(f);
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(f);
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "mcp_relay_tests.rs"]
mod tests;
