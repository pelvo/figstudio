//! Browser RPC jobs: immutable snapshots cross the pool boundary, UI commands return by channel.
use crate::{
    mcp_state::viewport,
    mcp_wire::{busy_reply, encode_failed_reply, rpc_reply},
    tab::Tab,
};
use figstudio_fig::Guid;
use figstudio_mcp_core::{
    CoreError,
    rendering::Rendering,
    wasm_rpc::{McpTab, McpTabEnv, handle_request},
};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
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
        /// The reveal's claim state (see [`claim_reveal`]); the UI must win it before
        /// applying, the worker can expire it.
        claim: Arc<AtomicU8>,
    },
    Reply {
        generation: u32,
        text: String,
    },
}

/// A reveal's handshake state. The UI must claim it before applying; the worker can
/// expire it when its bounded wait ends. Whichever side wins the atomic first decides, so
/// the UI never applies a reveal the caller was already told failed — and once the UI has
/// claimed, its apply+ack is one synchronous frame block, so a worker that sees the claim
/// knows the ack is imminent and reports the real outcome instead of a timeout.
const REVEAL_PENDING: u8 = 0;
const REVEAL_CLAIMED: u8 = 1;
const REVEAL_EXPIRED: u8 = 2;

/// How long a worker waits for the UI to apply a reveal before failing the call. Blocking
/// is safe here: this runs on the Rayon pool, never the UI thread, and the UI frame loop
/// keeps polling the inbox while replies are pending. The wait polls because std's
/// `Instant` (and so `recv_timeout`) is unsupported on wasm32; `web_time` and
/// `thread::sleep` work on the pool workers (as `worker.rs` already relies on).
const REVEAL_ACK_TIMEOUT: Duration = Duration::from_secs(2);
const ACK_POLL: Duration = Duration::from_millis(4);

/// In-flight relay jobs this tab runs at once. Each holds a snapshot and a Rayon worker
/// the canvas also needs, so a burst the worker admitted (up to 128 per room) must not
/// stack renders here; the refused call is answered with a busy error instead.
const MAX_INFLIGHT: usize = 8;

#[derive(Debug)]
pub(crate) struct Inbox {
    tx: Sender<Done>,
    pub rx: Receiver<Done>,
    pub pending: usize,
    /// Test seam for [`REVEAL_ACK_TIMEOUT`]: dispatch copies it into each snapshot.
    ack_timeout: Duration,
}

impl Default for Inbox {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            pending: 0,
            ack_timeout: REVEAL_ACK_TIMEOUT,
        }
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
        let claim = Arc::new(AtomicU8::new(REVEAL_PENDING));
        self.tx
            .send(Done::Reveal {
                generation: self.generation,
                tab,
                node,
                ack,
                claim: claim.clone(),
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
            // Expire only while the reveal is still unclaimed: a claim means the UI is
            // applying it right now (one synchronous block) and the ack is imminent, so
            // keep waiting and report the real outcome. The UI never applies an expired
            // reveal, so the caller's answer and the viewport cannot disagree.
            if Instant::now() >= deadline
                && claim
                    .compare_exchange(
                        REVEAL_PENDING,
                        REVEAL_EXPIRED,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
            {
                return Err(CoreError::input(
                    "the browser tab did not apply the reveal within 2 s",
                ));
            }
            // Run queued pool work while waiting, as web.rs does: with one pool thread a
            // bare sleep would stall renders for the whole ack window.
            #[cfg(target_arch = "wasm32")]
            let ran = rayon::yield_now() == Some(rayon::Yield::Executed);
            #[cfg(not(target_arch = "wasm32"))]
            let ran = false;
            if !ran {
                std::thread::sleep(ACK_POLL);
            }
        }
    }
}

/// The UI thread's side of a reveal (run by `McpHost::frame`): try to claim it before
/// touching any state. `false` means the worker already expired it and told the caller it
/// was not applied, so the UI must leave the viewport alone.
pub(crate) fn claim_reveal(claim: &AtomicU8) -> bool {
    claim
        .compare_exchange(
            REVEAL_PENDING,
            REVEAL_CLAIMED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

/// Apply a claimed reveal: the mutation itself, or why it cannot run, so the worker's
/// bounded wait always ends. `live` is the connection's generation, or `None` when no
/// relay connection is open.
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
    /// Queue one relay call on the pool. Returns the busy frame to send when the tab is
    /// already running [`MAX_INFLIGHT`] jobs (the call is refused, not stacked).
    pub fn dispatch(
        &mut self,
        generation: u32,
        id: String,
        body: serde_json::Value,
        tabs: &[Tab],
        active: usize,
    ) -> Option<String> {
        // The relay forwards requests only with an id, so `body.id` is the caller's.
        let rpc_id = body.get("id").cloned().unwrap_or(serde_json::Value::Null);
        if self.pending >= MAX_INFLIGHT {
            return Some(busy_reply(&id, &rpc_id));
        }
        self.pending += 1;
        let keys = crate::mcp_wire::relay_keys(tabs);
        // This runs on the UI thread, which must not wait on the workers' font lock.
        #[cfg(target_arch = "wasm32")]
        let fonts = crate::canvas_fonts::latest();
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
                    #[cfg(target_arch = "wasm32")]
                    let rendering = {
                        let mut rendering = rendering;
                        rendering.fonts = fonts.clone();
                        rendering
                    };
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
            ack_timeout: self.ack_timeout,
        };
        spawn(move || {
            let result = handle_request(&body.to_string(), &env);
            // The dispatcher returns serialized JSON; malformed output still completes the
            // job, answering the caller (with their id) rather than leaving the 30 s timer
            // to expire.
            let text = rpc_reply(&id, result.as_deref())
                .unwrap_or_else(|_| encode_failed_reply(&id, &rpc_id));
            let _ = env.tx.send(Done::Reply { generation, text });
        });
        None
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
