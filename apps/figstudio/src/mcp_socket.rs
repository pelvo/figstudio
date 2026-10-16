//! Socket callbacks only enqueue events; no browser file reads or RPC run here.
use crate::mcp_upload::{Event, Shared};
use crate::mcp_wire::{CONNECT_TIMEOUT_MS, parse_rpc};
use figstudio_mcp_core::bridge::ServerMessage;
use std::rc::Rc;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{CloseEvent, MessageEvent, WebSocket};

pub(super) struct Conn {
    pub ws: WebSocket,
    pub welcomed: bool,
    _handlers: (
        Closure<dyn FnMut(MessageEvent)>,
        Closure<dyn FnMut(CloseEvent)>,
        Closure<dyn FnMut(web_sys::Event)>,
    ),
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

impl Conn {
    /// Open the socket. The relay bridge offers exactly two subprotocols — the constant
    /// marker and the raw executor hex (`ExecutorPair::subprotocols`) — so the executor
    /// secret travels only in the WebSocket handshake, never in a URL. The local bridge
    /// (`protocols` is `None`) is unchanged.
    pub fn open(
        url: &str,
        protocols: Option<[String; 2]>,
        relay: bool,
        generation: u32,
        shared: &Rc<Shared>,
        ctx: &egui::Context,
    ) -> Result<Self, String> {
        let ws = match protocols {
            Some([marker, executor]) => WebSocket::new_with_str_sequence(
                url,
                &js_sys::Array::of2(
                    &wasm_bindgen::JsValue::from(marker.as_str()),
                    &wasm_bindgen::JsValue::from(executor.as_str()),
                ),
            ),
            None => WebSocket::new(url),
        }
        .map_err(|_| "Not a valid WebSocket address".to_owned())?;
        let push = |shared: &Rc<Shared>, ctx: &egui::Context, event: Event| {
            shared.events.borrow_mut().push_back(event);
            ctx.request_repaint();
        };
        let (s, c) = (shared.clone(), ctx.clone());
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let Some(text) = e.data().as_string() else {
                return;
            };
            if relay {
                // The keepalive answer is the exact auto-response frame (the Worker's
                // `setWebSocketAutoResponse`, web/worker.mjs) — checked before, and
                // instead of, `rpc` frame parsing, and tagged with this connection's
                // generation so only its own `pong` can clear its wait.
                if text == "pong" {
                    push(&s, &c, Event::Pong { generation });
                } else if let Some((id, body)) = parse_rpc(&text) {
                    push(&s, &c, Event::Rpc { id, body });
                }
            } else if let Ok(message) = serde_json::from_str::<ServerMessage>(&text) {
                push(&s, &c, Event::Server(message));
            }
        });
        let (s, c) = (shared.clone(), ctx.clone());
        let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |e: CloseEvent| {
            push(&s, &c, Event::Closed(e.code()));
        });
        let (s, c) = (shared.clone(), ctx.clone());
        let on_open =
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_| push(&s, &c, Event::Opened));
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        let (s, c) = (shared.clone(), ctx.clone());
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
        Ok(Self {
            ws,
            welcomed: false,
            _handlers: (on_message, on_close, on_open),
            timeout: timer.map(|id| (id, give_up)),
        })
    }
}
