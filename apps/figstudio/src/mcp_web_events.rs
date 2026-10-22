//! Apply socket events and dispatch relay work from the UI thread.
use super::*;
impl McpHost {
    pub(super) fn handle(
        &mut self,
        event: Event,
        tabs: &mut [Tab],
        active: &mut usize,
    ) -> Option<String> {
        match event {
            // A call the socket callback could not start yet (it came before a frame saw
            // the connection welcomed): start it against this frame's tabs. One from a
            // replaced socket is dropped (the relay failed it when that socket closed); one
            // that still cannot start waits for the next pass instead of being lost.
            Event::Rpc {
                generation,
                id,
                body,
            } => {
                if generation != self.generation {
                    return None;
                }
                self.publish(tabs, *active);
                let started = self
                    .calls
                    .borrow_mut()
                    .start(generation, id.clone(), body.clone());
                if !started && self.conn.is_some() {
                    self.later.push(Event::Rpc {
                        generation,
                        id,
                        body,
                    });
                }
                None
            }
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
            Event::Opened => {
                let ws = self.conn.as_ref()?.ws.clone();
                if self.relay {
                    let (Some(device_id), Some(instance)) = (&self.device_id, &self.instance)
                    else {
                        self.stop();
                        return Some("Could not generate a secure MCP device identity".to_owned());
                    };
                    let wire_label = room::current_device_label();
                    self.conn.as_mut()?.welcomed = true;
                    let focused = room::document_focused();
                    let frame = crate::mcp_wire::relay_ready_v2(
                        device_id,
                        &wire_label,
                        instance,
                        focused,
                        tabs,
                    );
                    self.last_key = Some(key(tabs, *active));
                    self.last_focused = Some(focused);
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
                if self.relay {
                    let quiet = self.reconnect.retrying();
                    self.stop();
                    self.reconnect.lost(Instant::now());
                    if quiet {
                        return None;
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
                // Read before `reset_link` starts a fresh heartbeat.
                let proven = self.heartbeat.proven();
                self.reset_link();
                if self.relay {
                    // A failed attempt of a retry series (it never answered a keepalive,
                    // even if its handshake opened) is retried again without a toast.
                    let quiet = !proven && self.reconnect.retrying();
                    self.calls.borrow_mut().publish(None, &[], 0);
                    self.reconnect.lost(Instant::now());
                    if quiet {
                        return None;
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
}
