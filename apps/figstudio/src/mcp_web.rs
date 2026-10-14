//! Agent access in the browser: pairs this tab with a local `figstudio-mcp --bridge`
//! process over a WebSocket (docs/WEB-MCP-PLAN.md). Same API as `mcp_host.rs`.
//!
//! The tab uploads each open file in slices, reports its view when that changes (the same
//! `Key` the desktop uses) and applies `reveal` requests. Off until the user connects or the
//! URL fragment carries `#mcp=<token>`; the document stays open if the bridge goes away.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use egui::RichText;
use figstudio_mcp_core::bridge::{ClientMessage, PROTOCOL_VERSION, ServerMessage};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{CloseEvent, MessageEvent, WebSocket};

use crate::mcp_state::{Key, key};
use crate::mcp_upload::{Event, Shared, send, upload};
use crate::mcp_wire::{
    CONNECT_TIMEOUT_MS, close_reason, closed_message, connect_failed_message, reveal_wire,
    state_message, token_from_fragment, upload_name,
};
use crate::tab::Tab;
use crate::theme;

const DEFAULT_URL: &str = "ws://127.0.0.1:3845/bridge";

/// A live socket; its callbacks are detached when it is dropped.
struct Conn {
    ws: WebSocket,
    welcomed: bool,
    _handlers: (
        Closure<dyn FnMut(MessageEvent)>,
        Closure<dyn FnMut(CloseEvent)>,
        Closure<dyn FnMut(web_sys::Event)>,
    ),
    /// The connect timeout: cleared and dropped with the connection, never leaked.
    timeout: Option<(i32, Closure<dyn FnMut()>)>,
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.ws.set_onmessage(None);
        self.ws.set_onclose(None);
        self.ws.set_onopen(None);
        let _ = self.ws.close();
        if let (Some((id, _)), Some(window)) = (&self.timeout, web_sys::window()) {
            window.clear_timeout_with_handle(*id);
        }
    }
}

pub struct McpHost {
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
}

impl Default for McpHost {
    fn default() -> Self {
        let hash = web_sys::window().and_then(|w| w.location().hash().ok());
        let token = hash.as_deref().and_then(token_from_fragment);
        let new_token = Rc::<RefCell<Option<String>>>::default();
        if let Some(window) = web_sys::window() {
            // A new `#mcp=` (say, a fresh `figstudio-mcp --bridge` run) pairs again.
            let (cell, w) = (new_token.clone(), window.clone());
            let on_hash = Closure::<dyn FnMut()>::new(move || {
                let hash = w.location().hash().ok();
                *cell.borrow_mut() = hash.as_deref().and_then(token_from_fragment);
            });
            let _ = window
                .add_event_listener_with_callback("hashchange", on_hash.as_ref().unchecked_ref());
            // The listener lives as long as the page does.
            on_hash.forget();
        }
        Self {
            url: DEFAULT_URL.to_owned(),
            want_connect: token.is_some(),
            token: token.unwrap_or_default(),
            conn: None,
            shared: Rc::default(),
            names: HashMap::new(),
            ready: HashSet::new(),
            failed: HashSet::new(),
            uploading: None,
            generation: 0,
            last_key: None,
            notices: Vec::new(),
            new_token,
        }
    }
}

impl McpHost {
    pub fn menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Agent access (local MCP)…", |ui| {
            ui.label("Pairs this tab with `figstudio-mcp --bridge` on this computer.");
            ui.add(egui::TextEdit::singleline(&mut self.url).hint_text(DEFAULT_URL));
            ui.add(
                egui::TextEdit::singleline(&mut self.token)
                    .password(true)
                    .hint_text("pairing token"),
            );
            if self.conn.is_none() {
                if ui.button("Connect").clicked() {
                    self.want_connect = true;
                    ui.close();
                }
            } else if ui.button("Disconnect").clicked() {
                self.stop();
                ui.close();
            }
        });
    }

    pub fn status(&self, ui: &mut egui::Ui) {
        let Some(conn) = &self.conn else { return };
        let text = match (conn.welcomed, self.shared.progress.get()) {
            (false, _) => "MCP connecting…".to_owned(),
            (true, Some((sent, total))) => format!("MCP uploading {}%", sent * 100 / total.max(1)),
            (true, None) => format!("MCP connected · {} tabs", self.ready.len()),
        };
        ui.label(RichText::new(text).color(theme::TEXT_DIM))
            .on_hover_text("This tab is paired with the local MCP bridge.");
        ui.separator();
    }

    /// Start of each frame: apply what the bridge sent. Returns a message for a toast.
    pub fn frame(&mut self, tabs: &mut [Tab], active: &mut usize) -> Option<String> {
        let mut toast = self.notices.pop();
        let events: Vec<Event> = self.shared.events.borrow_mut().drain(..).collect();
        for event in events {
            if let Some(message) = self.handle(event, tabs, active) {
                toast.get_or_insert(message);
            }
        }
        toast
    }

    fn handle(&mut self, event: Event, tabs: &mut [Tab], active: &mut usize) -> Option<String> {
        match event {
            Event::Opened => {
                let ws = self.conn.as_ref()?.ws.clone();
                let hello = ClientMessage::Hello {
                    token: self.token.clone(),
                    version: PROTOCOL_VERSION,
                };
                send(&ws, &hello)
                    .err()
                    .map(|_| "Could not talk to the MCP bridge".to_owned())
            }
            Event::Server(ServerMessage::Welcome { .. }) => {
                self.conn.as_mut()?.welcomed = true;
                None
            }
            Event::ConnectTimeout(generation) => {
                let waiting = self.generation == generation
                    && self.conn.as_ref().is_some_and(|c| !c.welcomed);
                if !waiting {
                    return None;
                }
                self.stop();
                Some(connect_failed_message().to_owned())
            }
            Event::Server(ServerMessage::Error { message, tab }) => {
                // Only upload failures name a tab; other errors leave the tabs alone.
                if let Some(tab) = tab {
                    self.ready.remove(&tab);
                    self.failed.insert(tab);
                    // Stop a running upload: the bridge drops its remaining frames anyway.
                    self.shared.cancelled.borrow_mut().insert(tab);
                    self.last_key = None;
                }
                Some(format!("MCP bridge: {message}"))
            }
            Event::Server(ServerMessage::Reveal { id, tab, node }) => {
                let result = reveal_wire(tabs, active, tab, &node);
                let ws = self.conn.as_ref()?.ws.clone();
                let _ = send(
                    &ws,
                    &ClientMessage::Reply {
                        id,
                        error: result.err(),
                    },
                );
                None
            }
            Event::Closed(_) if self.conn.is_none() => None,
            Event::Closed(code) => {
                let welcomed = self.conn.take().is_some_and(|c| c.welcomed);
                self.reset_link();
                Some(closed_message(welcomed, code).to_owned())
            }
            Event::Uploaded { generation, .. } if generation != self.generation => None,
            Event::Uploaded { tab, result, .. } => {
                self.uploading = None;
                match result {
                    Ok(()) if self.failed.contains(&tab) => None,
                    Ok(()) => {
                        self.ready.insert(tab);
                        self.last_key = None;
                        None
                    }
                    Err(error) => {
                        // Already failed: the bridge's error was shown and cancelled this upload.
                        let reported = !self.failed.insert(tab);
                        // The bridge may hold a partial file for this tab: tell it to drop it.
                        // A tab that was closed already got `close` from `drop_closed`.
                        if self.names.contains_key(&tab)
                            && let Some(conn) = &self.conn
                        {
                            let _ = send(&conn.ws, &ClientMessage::Close { tab });
                        }
                        (self.names.contains_key(&tab) && !reported)
                            .then(|| format!("MCP upload failed: {error}"))
                    }
                }
            }
        }
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
        let fragment = self.new_token.borrow_mut().take();
        if let Some(token) = fragment.filter(|t| *t != self.token) {
            // Drop the old socket; `connect` below resets the link state.
            self.conn = None;
            self.token = token;
            self.want_connect = true;
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
        if self.token.is_empty() {
            return Err("Enter the pairing token printed by figstudio-mcp --bridge".to_owned());
        }
        let ws =
            WebSocket::new(&self.url).map_err(|_| "Not a valid WebSocket address".to_owned())?;
        self.generation = self.generation.wrapping_add(1);
        let push = |shared: &Rc<Shared>, ctx: &egui::Context, event: Event| {
            shared.events.borrow_mut().push_back(event);
            ctx.request_repaint();
        };
        let (s, c) = (self.shared.clone(), ctx.clone());
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            // Binary or unparseable frames are ignored: nothing here trusts the peer.
            let parsed = e
                .data()
                .as_string()
                .and_then(|t| serde_json::from_str::<ServerMessage>(&t).ok());
            if let Some(message) = parsed {
                push(&s, &c, Event::Server(message));
            }
        });
        let (s, c) = (self.shared.clone(), ctx.clone());
        let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |e: CloseEvent| {
            push(&s, &c, Event::Closed(e.code()))
        });
        let (s, c) = (self.shared.clone(), ctx.clone());
        let on_open =
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_| push(&s, &c, Event::Opened));
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        // A socket that never says `welcome` (Local Network Access, mixed content) closes
        // with no message of its own: give up after a while.
        let (s, c, generation) = (self.shared.clone(), ctx.clone(), self.generation);
        let give_up = Closure::<dyn FnMut()>::once(move || {
            push(&s, &c, Event::ConnectTimeout(generation));
        });
        let timer = web_sys::window().and_then(|window| {
            window
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    give_up.as_ref().unchecked_ref(),
                    CONNECT_TIMEOUT_MS,
                )
                .ok()
        });
        self.reset_link();
        self.shared.events.borrow_mut().clear();
        self.conn = Some(Conn {
            ws,
            welcomed: false,
            _handlers: (on_message, on_close, on_open),
            timeout: timer.map(|id| (id, give_up)),
        });
        Ok(())
    }

    /// Close the socket; open documents stay.
    pub fn stop(&mut self) {
        self.conn = None;
        self.reset_link();
        self.shared.events.borrow_mut().clear();
    }
}
