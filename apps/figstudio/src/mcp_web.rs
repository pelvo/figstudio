//! Browser agent access: same-origin Wasm MCP relay, or the opt-in local upload bridge.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use egui::RichText;
use figstudio_mcp_core::bridge::{ClientMessage, PROTOCOL_VERSION, ServerMessage};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::WebSocket;

use crate::mcp_state::{Key, key};
use crate::mcp_upload::{Event, Shared, send, upload};
use crate::mcp_wire::{
    closed_message, connect_failed_message, reveal_wire, room_from_fragment, state_message,
    token_from_fragment, upload_name,
};
use crate::tab::Tab;
use crate::theme;

const DEFAULT_URL: &str = "ws://127.0.0.1:3845/bridge";

#[path = "mcp_web_events.rs"]
mod events;
#[path = "mcp_web_menu.rs"]
mod menu;
#[path = "mcp_room.rs"]
mod room;
#[path = "mcp_socket.rs"]
mod socket;
use socket::Conn;

pub struct McpHost {
    relay: bool,
    room: String,
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
    /// A token the URL fragment gained after startup (`hashchange`); `sync` applies it.
    new_token: Rc<RefCell<Option<String>>>,
    /// A room a `#room=` fragment named after startup; `sync` applies it.
    new_room: Rc<RefCell<Option<String>>>,
    /// The `Use room token` field; applied by `join_room` in the menu.
    room_input: String,
}

impl Default for McpHost {
    fn default() -> Self {
        let hash = web_sys::window().and_then(|w| w.location().hash().ok());
        let token = hash.as_deref().and_then(token_from_fragment);
        let new_token = Rc::<RefCell<Option<String>>>::default();
        let new_room = Rc::<RefCell<Option<String>>>::default();
        if let Some(window) = web_sys::window() {
            // A new `#mcp=` (say, a fresh `figstudio-mcp --bridge` run) pairs again, and a
            // new `#room=` joins another browser's room.
            let (token_cell, room_cell, w) = (new_token.clone(), new_room.clone(), window.clone());
            let on_hash = Closure::<dyn FnMut()>::new(move || {
                let hash = w.location().hash().ok();
                *token_cell.borrow_mut() = hash.as_deref().and_then(token_from_fragment);
                *room_cell.borrow_mut() = hash.as_deref().and_then(room_from_fragment);
            });
            let _ = window
                .add_event_listener_with_callback("hashchange", on_hash.as_ref().unchecked_ref());
            // The listener lives as long as the page does.
            on_hash.forget();
        }
        let mut notices: Vec<String> = Vec::new();
        // A `#room=` link joins that room; a legacy `#mcp=` pairing fragment wins over it.
        let linked_room = token
            .is_none()
            .then(|| hash.as_deref().and_then(room_from_fragment))
            .flatten();
        let room = match linked_room {
            Some(room) => {
                // The join applies even when persistence is denied; the notice says so.
                if let Err(error) = room::save(&room) {
                    notices.push(error);
                }
                room::clear_fragment();
                room
            }
            None => {
                let loaded = room::load();
                notices.extend(loaded.as_ref().err().cloned());
                loaded.unwrap_or_default()
            }
        };
        let relay = token.is_none() && room::urls(&room).is_some();
        Self {
            relay,
            room,
            rpc: Default::default(),
            url: DEFAULT_URL.to_owned(),
            want_connect: relay || token.is_some(),
            token: token.unwrap_or_default(),
            conn: None,
            shared: Rc::default(),
            names: HashMap::new(),
            ready: HashSet::new(),
            failed: HashSet::new(),
            uploading: None,
            generation: 0,
            last_key: None,
            notices,
            new_token,
            new_room,
            room_input: String::new(),
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
                } => {
                    // Always answer: a worker is blocked on this ack with a bounded wait.
                    let live = (self.conn.is_some() && self.relay).then_some(self.generation);
                    let _ = ack.send(crate::mcp_relay::apply_reveal(
                        live, generation, tabs, active, tab, node,
                    ));
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
        self.shared.progress.set(None);
        self.shared.cancelled.borrow_mut().clear();
    }

    /// End of each frame: connect when asked, upload new tabs, tell the bridge about closed
    /// tabs and publish the view when it changed.
    pub fn sync(&mut self, ctx: &egui::Context, tabs: &[Tab], active: usize) {
        // Workers cannot wake egui from Wasm; poll while replies/image reads are pending,
        // even after disconnect, so snapshots finish and release their sources.
        if self.rpc.pending > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let fragment = self.new_token.borrow_mut().take();
        let linked = self.new_room.borrow_mut().take();
        if let Some(token) = fragment.as_deref().filter(|t| *t != self.token) {
            // Drop the old socket; `connect` below resets the link state.
            self.conn = None;
            self.token = token.to_owned();
            self.relay = false;
            self.want_connect = true;
        } else if fragment.is_none()
            && let Some(room) = linked
        {
            // The link joins that room here; consume the fragment so the token does not
            // stay in the address bar, the history or a copied URL.
            room::clear_fragment();
            if room != self.room {
                if let Err(error) = room::save(&room) {
                    self.notices.push(error);
                }
                self.conn = None;
                self.room = room;
                self.relay = true;
                self.want_connect = true;
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
            if self.last_key.as_ref() != Some(&key) {
                let _ = ws.send_with_str(crate::mcp_wire::relay_ready());
                self.last_key = Some(key);
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
        self.generation = self.generation.wrapping_add(1);
        self.reset_link();
        self.shared.events.borrow_mut().clear();
        let url = if self.relay {
            room::urls(&self.room)
                .ok_or_else(|| "Cloud relay needs an HTTP(S) page".to_owned())?
                .0
        } else {
            self.url.clone()
        };
        self.conn = Some(Conn::open(
            &url,
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
}
