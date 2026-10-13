//! Streams one open file to the local MCP bridge in slices (browser only). Big files are
//! never held whole: each slice is read from the browser `File`, sent, and dropped.
use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;

use figstudio_mcp_core::bridge::{ClientMessage, ServerMessage};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::WebSocket;

use crate::mcp_wire::{CHUNK, chunk_ranges};

/// Stop reading ahead while this much is still queued in the socket.
const MAX_BUFFERED: u32 = 4 * 1024 * 1024;

/// What socket callbacks and upload tasks tell the host; drained once per frame.
pub(crate) enum Event {
    Opened,
    /// No `welcome` in time for connection `generation` (the host ignores a stale one).
    ConnectTimeout(u32),
    Server(ServerMessage),
    Closed(u16),
    Uploaded {
        generation: u32,
        tab: u64,
        result: Result<(), String>,
    },
}

/// State shared by callbacks, upload tasks and the host (single-threaded, so `Rc`).
#[derive(Default)]
pub(crate) struct Shared {
    pub events: RefCell<VecDeque<Event>>,
    /// `(sent, total)` bytes of the running upload.
    pub progress: Cell<Option<(u64, u64)>>,
    /// Tabs closed or rejected while uploading: the task stops at the next slice.
    pub cancelled: RefCell<HashSet<u64>>,
}

pub(crate) fn send(ws: &WebSocket, message: &ClientMessage) -> Result<(), String> {
    let text = serde_json::to_string(message).map_err(|e| e.to_string())?;
    ws.send_with_str(&text)
        .map_err(|e| crate::mcp_wire::js_error_text(&e))
}

async fn nap(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// Uploads `file` as tab `tab`, then reports `Event::Uploaded` for connection `generation`
/// (the host ignores reports from a connection it has since dropped).
pub(crate) async fn upload(
    ws: WebSocket,
    shared: Rc<Shared>,
    ctx: egui::Context,
    generation: u32,
    tab: u64,
    name: String,
    file: web_sys::File,
) {
    let result = run(&ws, &shared, &ctx, tab, name, &file).await;
    shared.progress.set(None);
    let event = Event::Uploaded {
        generation,
        tab,
        result,
    };
    shared.events.borrow_mut().push_back(event);
    ctx.request_repaint();
}

async fn run(
    ws: &WebSocket,
    shared: &Shared,
    ctx: &egui::Context,
    tab: u64,
    name: String,
    file: &web_sys::File,
) -> Result<(), String> {
    let size = file.size() as u64;
    send(ws, &ClientMessage::Open { tab, name, size })?;
    let mut sent = 0;
    for (at, len) in chunk_ranges(size) {
        while ws.buffered_amount() > MAX_BUFFERED {
            if ws.ready_state() != WebSocket::OPEN {
                return Err("bridge disconnected".into());
            }
            nap(20).await;
        }
        let bytes = crate::web::read_slice(file, at, len, CHUNK as usize).await?;
        // Checked after the read: the tab may have closed while it ran.
        if shared.cancelled.borrow().contains(&tab) {
            return Err("upload cancelled".into());
        }
        // A view of Wasm memory is shared, and `WebSocket.send` rejects shared views: copy
        // the slice into a fresh buffer first.
        let buffer = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
        buffer.copy_from(&bytes);
        ws.send_with_array_buffer(&buffer.buffer())
            .map_err(|e| crate::mcp_wire::js_error_text(&e))?;
        sent += len;
        shared.progress.set(Some((sent, size)));
        ctx.request_repaint();
    }
    if shared.cancelled.borrow().contains(&tab) {
        return Err("upload cancelled".into());
    }
    send(ws, &ClientMessage::End { tab })
}
