//! Browser agent access: same-origin Wasm MCP relay, or the opt-in local upload bridge.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use egui::RichText;
use figstudio_mcp_core::bridge::{ClientMessage, PROTOCOL_VERSION, ServerMessage};
use web_sys::WebSocket;
use web_time::Instant;

use crate::mcp_heartbeat::{Beat, Heartbeat, PONG_DEADLINE};
use crate::mcp_join::{Link, RoomProposal, parse_link, proposed_pair};
use crate::mcp_pair::ExecutorPair;
use crate::mcp_state::{Key, key};
use crate::mcp_upload::{Event, Shared, send, upload};
use crate::mcp_wire::{
    closed_message, connect_failed_message, reveal_wire, state_message, upload_name,
};
use crate::tab::Tab;
use crate::theme;

const DEFAULT_URL: &str = "ws://127.0.0.1:3845/bridge";

#[path = "mcp_web_events.rs"]
mod events;
#[path = "mcp_web_join.rs"]
mod join;
#[path = "mcp_web_menu.rs"]
mod menu;
#[path = "mcp_room.rs"]
mod room;
#[path = "mcp_socket.rs"]
mod socket;
use socket::Conn;

pub struct McpHost {
    relay: bool,
    /// The executor credential; `None` until storage yields or accepts one.
    pair: Option<ExecutorPair>,
    rpc: crate::mcp_relay::Inbox,
    url: String,
    token: String,
    /// Set by the menu button or the URL fragment; consumed by `sync` (it has the context).
    want_connect: bool,
    conn: Option<Conn>,
    shared: Rc<Shared>,
    /// Open tabs by id, and the browser file name each was opened under.
    names: HashMap<u64, PathBuf>,
    /// Tabs the bridge has fully received.
    ready: HashSet<u64>,
    /// Tabs that could not be uploaded; not retried until reconnect.
    failed: HashSet<u64>,
    uploading: Option<u64>,
    /// Bumped per connection so a stale upload task's report is ignored.
    generation: u32,
    last_key: Option<Key>,
    /// Toasts raised outside `frame` (shown by the next `frame`).
    notices: Vec<String>,
    /// The fragment as last seen by `hashchange`; `sync` reads it every frame.
    link: Rc<RefCell<Option<Link>>>,
    /// A linked room waiting for the user's answer; never saved or connected before it.
    proposal: Option<RoomProposal>,
    /// Local `--browser-proxy` target (`ws://127.0.0.1:<port>/bridge` with Wasm RPC).
    proxy: Option<crate::mcp_join::ProxyTarget>,
    /// The `Use room token` field; applied by `Apply` in the menu.
    room_input: String,
    /// Persistent device identifier (`figstudio.mcp.device.v1`); `None` if entropy failed.
    device_id: Option<String>,
    /// User-editable device label (`figstudio.mcp.device_label.v1`).
    device_label: String,
    /// Per-page-load instance identifier separating tabs on the same device.
    instance: Option<String>,
    /// Last focus state reported to the relay.
    last_focused: Option<bool>,
    /// Relay keepalive state: a ping's pong must come back within a deadline or the
    /// connection is dropped (see `mcp_heartbeat.rs`).
    heartbeat: Heartbeat<Instant>,
}

impl Default for McpHost {
    fn default() -> Self {
        let hash = web_sys::window().and_then(|w| w.location().hash().ok());
        let link = parse_link(hash.as_deref());
        let link_cell = Rc::<RefCell<Option<Link>>>::default();
        join::watch_fragment(link_cell.clone());
        // A `#room=` key is consumed the moment it is read, whatever else the fragment says.
        if let Some(stripped) = &link.stripped {
            room::replace_fragment(stripped);
        }
        let mut notices: Vec<String> = Vec::new();
        let loaded = room::load();
        notices.extend(loaded.notices);
        let pair = loaded.pair;
        let device = room::load_device();
        notices.extend(device.notices);
        // A link names a room to propose, not to use; the pair above stays this tab's own.
        let proposal =
            proposed_pair(link.pair, link.token.is_some(), pair.as_ref()).map(RoomProposal::new);
        let relay = link.token.is_none()
            && device.id.is_some()
            && device.instance.is_some()
            && (link.proxy.is_some()
                || pair
                    .as_ref()
                    .is_some_and(|p| room::urls(&p.room()).is_some()));
        Self {
            relay,
            pair,
            rpc: Default::default(),
            url: DEFAULT_URL.to_owned(),
            want_connect: relay || link.token.is_some(),
            token: link.token.unwrap_or_default(),
            conn: None,
            shared: Rc::default(),
            names: HashMap::new(),
            ready: HashSet::new(),
            failed: HashSet::new(),
            uploading: None,
            generation: 0,
            last_key: None,
            notices,
            link: link_cell,
            proposal,
            proxy: link.proxy,
            room_input: String::new(),
            device_id: device.id,
            device_label: device.label,
            instance: device.instance,
            last_focused: None,
            heartbeat: Heartbeat::fresh(0),
        }
    }
}

impl McpHost {
    /// Start of each frame: apply what the bridge sent. Returns a message for a toast.
    pub fn frame(&mut self, tabs: &mut [Tab], active: &mut usize) -> Option<String> {
        let mut toast = self.notices.pop();
        while let Ok(done) = self.rpc.rx.try_recv() {
            use crate::mcp_relay::Done;
            if matches!(done, Done::Reply { .. }) {
                self.rpc.pending = self.rpc.pending.saturating_sub(1);
            }
            match done {
                Done::Reveal {
                    generation,
                    tab,
                    node,
                    ack,
                    claim,
                } => {
                    // Claim before touching state: a worker that already expired this
                    // reveal answered the caller with a tool error, so a late frame must
                    // not apply it after that (see `claim_reveal`).
                    let result = if crate::mcp_relay::claim_reveal(&claim) {
                        let live = (self.conn.is_some() && self.relay).then_some(self.generation);
                        crate::mcp_relay::apply_reveal(live, generation, tabs, active, tab, node)
                    } else {
                        Err("the reveal expired before the UI applied it".to_owned())
                    };
                    // Always answer: a worker is blocked on this ack with a bounded wait.
                    let _ = ack.send(result);
                }
                Done::Reply { generation, text }
                    if self.conn.is_some() && self.relay && generation == self.generation =>
                {
                    if let Some(conn) = &self.conn {
                        let _ = conn.ws.send_with_str(&text);
                    }
                }
                _ => {}
            }
        }
        let events: Vec<Event> = self.shared.events.borrow_mut().drain(..).collect();
        for event in events {
            if let Some(message) = self.handle(event, tabs, active) {
                toast.get_or_insert(message);
            }
        }
        toast
    }

    /// Forget everything the bridge knew; a new connection uploads again.
    fn reset_link(&mut self) {
        self.ready.clear();
        self.failed.clear();
        self.uploading = None;
        self.last_key = None;
        self.last_focused = None;
        self.heartbeat = Heartbeat::fresh(self.generation);
        self.shared.progress.set(None);
        self.shared.cancelled.borrow_mut().clear();
    }

    /// End of each frame: connect when asked, upload new tabs, tell the bridge about closed
    /// tabs and publish the view when it changed.
    /// The browser has no folders to open a link's local copy from (`reveal_node` refuses it).
    pub fn take_links(&mut self) -> Vec<(String, crate::link_open::Done)> {
        Vec::new()
    }

    /// [`Self::sync`], then send the link answers: an MCP caller told a file is open finds
    /// it in the published snapshot.
    pub fn sync_then_answer(
        &mut self,
        ctx: &egui::Context,
        tabs: &[Tab],
        active: usize,
        answers: Vec<crate::link_open::Answer>,
    ) {
        self.sync(ctx, tabs, active);
        for (done, result) in answers {
            done(result);
        }
    }

    pub fn sync(&mut self, ctx: &egui::Context, tabs: &[Tab], active: usize) {
        // Workers cannot wake egui from Wasm; poll while replies/image reads are pending,
        // even after disconnect, so snapshots finish and release their sources.
        if self.rpc.pending > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let change = self.link.borrow_mut().take();
        if let Some(change) = change {
            // A `#room=` key is consumed the moment it is read, whatever else it said.
            if let Some(stripped) = &change.stripped {
                room::replace_fragment(stripped);
            }
            let pairing = change
                .token
                .as_deref()
                .filter(|token| *token != self.token)
                .is_some();
            if let Some(token) = change.token.filter(|token| *token != self.token) {
                // A fresh `#mcp=` pairing (say, a new `figstudio-mcp --bridge` run) moves
                // this tab to the local bridge; a pending link is dropped.
                self.conn = None;
                self.proxy = None;
                self.token = token;
                self.relay = false;
                self.want_connect = true;
                self.proposal = None;
            } else {
                if let Some(proxy) = change.proxy {
                    self.conn = None;
                    self.proxy = Some(proxy);
                    self.relay = true;
                    self.want_connect = true;
                }
                if let Some(pair) = proposed_pair(change.pair, pairing, self.pair.as_ref()) {
                    self.proposal = Some(RoomProposal::new(pair));
                    // The ask was staged at frame end; without this, an idle page never paints it.
                    ctx.request_repaint();
                }
            }
        }
        if std::mem::take(&mut self.want_connect) && self.conn.is_none() {
            if let Err(error) = self.connect(ctx) {
                self.notices.push(error);
            }
        }
        let Some(conn) = self.conn.as_ref().filter(|c| c.welcomed) else {
            return;
        };
        let ws = conn.ws.clone();
        self.drop_closed(&ws, tabs);
        if self.relay {
            // Release the picked-file upload handles when relay tabs close too.
            for tab in tabs {
                self.names.entry(tab.id).or_insert_with(|| tab.path.clone());
            }
            let key = key(tabs, active);
            let focused = room::document_focused();
            if self.last_key.as_ref() != Some(&key) || self.last_focused != Some(focused) {
                let wire_label = room::current_device_label();
                if let (Some(device_id), Some(instance)) = (&self.device_id, &self.instance) {
                    let frame = crate::mcp_wire::relay_ready_v2(
                        device_id,
                        &wire_label,
                        instance,
                        focused,
                        tabs,
                    );
                    let _ = ws.send_with_str(&frame);
                    self.last_key = Some(key);
                    self.last_focused = Some(focused);
                }
            }
            // Otherwise the relay socket can sit silent for a whole session while a NAT
            // or proxy has already forgotten the flow, and a successful send only proves
            // the browser buffered the bytes. The keepalive state machine runs the proof:
            // a ping goes out, the DO's runtime answers `pong` without waking it, and no
            // answer within the deadline drops the connection (see `mcp_heartbeat.rs`).
            // `Beat::Wait` schedules the repaint for exactly the next decision — not a
            // fixed cadence — so the deadline frame lands on time (or later, in a
            // throttled background tab; never early).
            let now = Instant::now();
            let beat = self.heartbeat.tick(now);
            match beat {
                Beat::Wait(resume) => ctx.request_repaint_after(resume),
                Beat::SendPing => match ws.send_with_str("ping") {
                    Ok(()) => {
                        self.heartbeat.ping_sent(now);
                        ctx.request_repaint_after(PONG_DEADLINE);
                    }
                    // A refused send is the browser already giving up on the socket:
                    // disconnect now, not after some later frame notices.
                    Err(_) => {
                        let msg = if self.proxy.is_some() {
                            "Local browser proxy disconnected"
                        } else {
                            "Cloud relay disconnected"
                        };
                        self.relay_lost(ctx, msg, self.heartbeat.proven())
                    }
                },
                // One automatic retry per proven connection; a connection that never
                // proved itself waits for the user (the storm bound).
                Beat::Elapsed { proven } => {
                    let msg = if self.proxy.is_some() {
                        "Local browser proxy stopped responding"
                    } else {
                        "Cloud relay stopped responding"
                    };
                    self.relay_lost(ctx, msg, proven)
                }
            }
            return;
        }
        if self.uploading.is_none() {
            self.start_upload(&ws, ctx, tabs);
        }
        let key = key(tabs, active);
        if self.last_key.as_ref() != Some(&key) {
            let _ = send(&ws, &state_message(tabs, active, &self.ready));
            self.last_key = Some(key);
        }
    }

    fn drop_closed(&mut self, ws: &WebSocket, tabs: &[Tab]) {
        let open: HashSet<u64> = tabs.iter().map(|t| t.id).collect();
        let closed: Vec<u64> = self
            .names
            .keys()
            .filter(|id| !open.contains(id))
            .copied()
            .collect();
        for id in closed {
            self.names.remove(&id);
            crate::web::forget_upload(id);
            self.shared.cancelled.borrow_mut().insert(id);
            if self.ready.remove(&id) || self.uploading == Some(id) {
                let _ = send(ws, &ClientMessage::Close { tab: id });
            }
            self.failed.remove(&id);
            self.last_key = None;
        }
    }

    fn start_upload(&mut self, ws: &WebSocket, ctx: &egui::Context, tabs: &[Tab]) {
        for tab in tabs {
            self.names.entry(tab.id).or_insert_with(|| tab.path.clone());
        }
        let next = tabs
            .iter()
            .find(|t| !self.ready.contains(&t.id) && !self.failed.contains(&t.id));
        let Some(tab) = next else { return };
        let Some(file) = crate::web::upload_file(tab.id) else {
            self.failed.insert(tab.id);
            return;
        };
        self.uploading = Some(tab.id);
        let name = upload_name(&tab.title());
        wasm_bindgen_futures::spawn_local(upload(
            ws.clone(),
            self.shared.clone(),
            ctx.clone(),
            self.generation,
            tab.id,
            name,
            file,
        ));
    }

    fn connect(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if !self.relay && self.token.is_empty() {
            return Err("Enter the pairing token printed by figstudio-mcp --bridge".to_owned());
        }
        if self.relay && self.proxy.is_none() && self.pair.is_none() {
            return Err("No MCP executor token yet; Apply one or Regenerate".to_owned());
        }
        if self.relay && (self.device_id.is_none() || self.instance.is_none()) {
            return Err("Could not generate a secure MCP device identity".to_owned());
        }
        self.generation = self.generation.wrapping_add(1);
        self.reset_link();
        self.shared.events.borrow_mut().clear();
        let (url, protocols) = if self.relay {
            if let Some(proxy) = &self.proxy {
                (proxy.ws_url(), Some(proxy.subprotocols()))
            } else {
                let pair = self
                    .pair
                    .as_ref()
                    .ok_or_else(|| "No MCP executor token".to_owned())?;
                let ws_url = room::urls(&pair.room())
                    .ok_or_else(|| "Cloud relay needs an HTTP(S) page".to_owned())?
                    .0;
                (ws_url, Some(pair.subprotocols()))
            }
        } else {
            (self.url.clone(), None)
        };
        self.conn = Some(Conn::open(
            &url,
            protocols,
            self.relay,
            self.generation,
            &self.shared,
            ctx,
        )?);
        Ok(())
    }

    /// Close the socket; open documents stay.
    pub fn stop(&mut self) {
        self.conn = None;
        self.reset_link();
        self.shared.events.borrow_mut().clear();
    }

    /// The relay connection is dead from our side: drop it, say why, and retry once
    /// when the connection had proven itself (`proven`); an unproven one waits for the
    /// user, so a black-holed path can never produce a retry storm. The repaint makes
    /// the next frame happen even on an otherwise idle page (it consumes `want_connect`
    /// and shows the notice).
    fn relay_lost(&mut self, ctx: &egui::Context, notice: &str, proven: bool) {
        self.stop();
        self.notices.push(notice.to_owned());
        if proven {
            self.want_connect = true;
        }
        ctx.request_repaint();
    }
}
