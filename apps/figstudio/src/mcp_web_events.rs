//! Apply socket events and dispatch relay work from the UI thread.
use super::*;
impl Host {
    pub(super) fn handle(
        &mut self,
        event: Event,
        tabs: &mut [Tab],
        active: &mut usize,
    ) -> Option<String> {
        let event = match self.link_event(event) {
            Ok(notice) => return notice,
            Err(event) => event,
        };
        match event {
            // A call the socket callback could not start yet (it came before the
            // connection was welcomed): start it against this frame's tabs. One that still
            // cannot start waits for the next pass instead of being lost.
            Event::Rpc {
                generation,
                id,
                body,
            } => {
                self.publish(tabs, *active);
                let waiting = self.start_call(generation, id, body);
                self.later.extend(waiting);
                None
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
            // Handled by `link_event`.
            Event::Pong { .. }
            | Event::Opened
            | Event::Server(ServerMessage::Welcome { .. })
            | Event::ConnectTimeout(_)
            | Event::Closed(_) => None,
        }
    }

    /// The connection's own events, which need no tabs, so a hidden tab handles them
    /// between frames too (`mcp_web_pump.rs`). Hands any other event back.
    pub(super) fn link_event(&mut self, event: Event) -> Result<Option<String>, Event> {
        Ok(match event {
            Event::Pong { generation } => {
                // Events are drained before `sync` ticks the deadline, so a pong this
                // frame sees always wins over a deadline that also passed this frame
                // (the throttled-tab ordering, see `mcp_heartbeat.rs`). The machine
                // ignores a foreign generation's pong by itself; the connection guard
                // covers a pong drained after `stop` cleared the queue.
                if self.conn.is_some() {
                    self.heartbeat.pong(generation, Instant::now());
                    if self.heartbeat.proven() {
                        self.reconnect.proven();
                    }
                }
                None
            }
            Event::Opened => self.opened(),
            Event::Server(ServerMessage::Welcome { .. }) => {
                if let Some(conn) = self.conn.as_mut() {
                    conn.welcomed = true;
                }
                None
            }
            Event::ConnectTimeout(generation) => {
                let waiting = self.generation == generation
                    && self.conn.as_ref().is_some_and(|c| !c.welcomed);
                if !waiting {
                    return Ok(None);
                }
                if self.relay {
                    let quiet = self.reconnect.retrying();
                    self.stop();
                    self.reconnect.lost(Instant::now(), js_sys::Math::random());
                    if quiet {
                        return Ok(None);
                    }
                } else {
                    self.stop();
                }
                Some(
                    if self.relay && self.proxy.is_some() {
                        "Could not connect to the local browser proxy"
                    } else if self.relay {
                        "Could not connect to the cloud relay"
                    } else {
                        connect_failed_message()
                    }
                    .to_owned(),
                )
            }
            Event::Closed(_) if self.conn.is_none() => None,
            Event::Closed(code) => {
                let welcomed = self.conn.take().is_some_and(|c| c.welcomed);
                // Read before `reset_link` starts a fresh heartbeat.
                let proven = self.heartbeat.proven();
                self.reset_link();
                if self.relay {
                    // A failed attempt of a retry series (it never answered a keepalive,
                    // even if its handshake opened) is retried again without a toast.
                    let quiet = !proven && self.reconnect.retrying();
                    self.calls.borrow_mut().live(None);
                    self.reconnect.lost(Instant::now(), js_sys::Math::random());
                    if quiet {
                        return Ok(None);
                    }
                }
                Some(
                    if self.relay && self.proxy.is_some() {
                        "Local browser proxy disconnected"
                    } else if self.relay {
                        "Cloud relay disconnected"
                    } else {
                        closed_message(welcomed, code)
                    }
                    .to_owned(),
                )
            }
            event => return Err(event),
        })
    }

    /// The socket opened: greet it (the relay's `ready`, the local bridge's `hello`).
    /// The relay is greeted with the tabs as the last frame saw them, so this needs no
    /// frame of its own.
    fn opened(&mut self) -> Option<String> {
        let ws = self.conn.as_ref()?.ws.clone();
        if self.relay {
            let (Some(device_id), Some(instance)) = (&self.device_id, &self.instance) else {
                self.stop();
                return Some("Could not generate a secure MCP device identity".to_owned());
            };
            let wire_label = room::current_device_label();
            let focused = room::document_focused();
            let frame = crate::mcp_wire::relay_ready_frame(
                device_id,
                &wire_label,
                instance,
                focused,
                &self.seen.1,
            );
            self.conn.as_mut()?.welcomed = true;
            self.last_key = Some(self.seen.0.clone());
            self.last_focused = Some(focused);
            // Calls that arrive before the next frame start at once.
            self.go_live();
            let err_msg = if self.proxy.is_some() {
                "Could not talk to the local browser proxy"
            } else {
                "Could not talk to the cloud relay"
            };
            return ws.send_with_str(&frame).err().map(|_| err_msg.to_owned());
        }
        let hello = ClientMessage::Hello {
            token: self.token.clone(),
            version: PROTOCOL_VERSION,
        };
        send(&ws, &hello)
            .err()
            .map(|_| "Could not talk to the MCP bridge".to_owned())
    }
}
