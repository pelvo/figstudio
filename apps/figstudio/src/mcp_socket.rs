//! Socket callbacks enqueue events and start relay calls on the pool; no browser file
//! reads run here. Each event also wakes the host, which handles the connection's own
//! events without waiting for a frame (`mcp_web_pump.rs`).
use super::Calls;
use crate::mcp_upload::{Event, Shared};
use crate::mcp_wire::{CONNECT_TIMEOUT_MS, parse_rpc};
use figstudio_mcp_core::bridge::ServerMessage;
use std::cell::RefCell;
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
        calls: &Rc<RefCell<Calls>>,
        wake: Rc<dyn Fn()>,
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
        // The wake handles the event (or leaves it for the frame) and asks for a frame.
        let push = {
            let wake = wake.clone();
            move |shared: &Rc<Shared>, event: Event| {
                shared.events.borrow_mut().push_back(event);
                wake();
            }
        };
        let (s, p, calls) = (shared.clone(), push.clone(), Rc::downgrade(calls));
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
                    p(&s, Event::Pong { generation });
                } else if let Some((id, body)) = parse_rpc(&text) {
                    // Started here, not at the next frame: a hidden tab may not draw one
                    // before the relay's deadline (`mcp_web_calls.rs`).
                    let started = calls.upgrade().is_some_and(|calls| {
                        calls
                            .borrow_mut()
                            .start(generation, id.clone(), body.clone())
                    });
                    if started {
                        // A frame still has work for a started call (a reveal is applied
                        // in one, and its ack waits only 2 s), and an idle page draws
                        // none unless asked.
                        wake();
                    } else {
                        p(
                            &s,
                            Event::Rpc {
                                generation,
                                id,
                                body,
                            },
                        );
                    }
                }
            } else if let Ok(message) = serde_json::from_str::<ServerMessage>(&text) {
                p(&s, Event::Server(message));
            }
        });
        let (s, p) = (shared.clone(), push.clone());
        let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |e: CloseEvent| {
            p(&s, Event::Closed(e.code()));
        });
        let (s, p) = (shared.clone(), push.clone());
        let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| p(&s, Event::Opened));
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        let s = shared.clone();
        let give_up = Closure::<dyn FnMut()>::once(move || {
            push(&s, Event::ConnectTimeout(generation));
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
