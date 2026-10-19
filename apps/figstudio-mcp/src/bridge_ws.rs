use crate::{
    bridge::Bridge,
    bridge_upload::Session,
    live::{Command, REVEAL_TIMEOUT, Snapshot},
};
use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use figstudio_mcp_core::bridge::{ClientMessage, PROTOCOL_VERSION, ServerMessage, guid_string};
use std::{
    collections::HashMap,
    sync::{Arc, PoisonError},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::oneshot;

/// A peer that stops reading must not hold the session (and the `owner` lock) forever.
const SEND_TIMEOUT: Duration = Duration::from_secs(1);
use tokio_util::sync::CancellationToken;

pub(crate) async fn upgrade(
    State(bridge): State<Arc<Bridge>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !headers
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| {
            bridge
                .config
                .origins
                .iter()
                .any(|allowed| allowed == origin)
        })
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    // Bound each frame independently of the file cap (uploads are chunked).
    ws.max_message_size(1024 * 1024)
        .max_frame_size(1024 * 1024)
        .on_upgrade(move |socket| session(socket, bridge))
}
async fn send(socket: &mut WebSocket, message: ServerMessage) -> Result<(), ()> {
    let text = serde_json::to_string(&message).map_err(|_| ())?;
    bounded(socket, Message::Text(text.into())).await
}
async fn bounded(socket: &mut WebSocket, message: Message) -> Result<(), ()> {
    match tokio::time::timeout(SEND_TIMEOUT, socket.send(message)).await {
        Ok(Ok(())) => Ok(()),
        _ => Err(()),
    }
}
async fn close(socket: &mut WebSocket, code: u16) {
    let frame = CloseFrame {
        code,
        reason: "bridge authentication failed".into(),
    };
    let _ = bounded(socket, Message::Close(Some(frame))).await;
}
struct Pending {
    reply: oneshot::Sender<anyhow::Result<()>>,
    started: Instant,
}
async fn session(mut socket: WebSocket, bridge: Arc<Bridge>) {
    let hello = tokio::time::timeout(REVEAL_TIMEOUT, socket.recv()).await;
    let (token, version) = match hello {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<ClientMessage>(&text) {
            Ok(ClientMessage::Hello { token, version }) => (token, version),
            _ => {
                close(&mut socket, 4401).await;
                return;
            }
        },
        _ => {
            close(&mut socket, 4401).await;
            return;
        }
    };
    if !bool::from(token.as_bytes().ct_eq(bridge.config.token.as_bytes())) {
        close(&mut socket, 4401).await;
        return;
    }
    if version != PROTOCOL_VERSION {
        close(&mut socket, 4426).await;
        return;
    }
    let cancel = CancellationToken::new();
    {
        let mut connection = bridge
            .connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(previous) = connection.replace(cancel.clone()) {
            previous.cancel();
        }
    }
    // Serialize publication and cleanup so a replaced socket cannot erase new state.
    let _owner = tokio::select! {
        _ = cancel.cancelled() => return,
        owner = bridge.owner.lock() => owner,
    };
    let mut session = match Session::new() {
        Ok(session) => session,
        Err(_) => {
            close(&mut socket, 1011).await;
            return;
        }
    };
    bridge.publish(Snapshot::default());
    if send(
        &mut socket,
        ServerMessage::Welcome {
            version: PROTOCOL_VERSION,
        },
    )
    .await
    .is_err()
    {
        return;
    }
    let mut pending: HashMap<u64, Pending> = HashMap::new();
    let mut next_id = 0;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    loop {
        tokio::select! {
            _ = cancel.cancelled() => { let _ = bounded(&mut socket, Message::Close(None)).await; break; }
            _ = bridge.shutdown.cancelled() => { let _ = bounded(&mut socket, Message::Close(None)).await; break; }
            _ = bridge.wake.notified() => {
                loop {
                    let command = bridge.commands.lock().unwrap_or_else(PoisonError::into_inner).try_recv();
                    let (tab, node, reply) = match command {
                        Ok(Command::Reveal { tab, node, reply }) => (tab, node, reply),
                        Ok(Command::OpenLink { reply, .. }) => { let _ = reply.send(Err(anyhow::anyhow!(NO_LOCAL_COPIES))); continue; }
                        Err(_) => break,
                    };
                    next_id += 1;
                    if send(&mut socket, ServerMessage::Reveal { id: next_id, tab, node: guid_string(node) }).await.is_err() {
                        let _ = reply.send(Err(anyhow::anyhow!("browser disconnected")));
                        cancel.cancel(); break;
                    }
                    pending.insert(next_id, Pending { reply, started: Instant::now() });
                }
            }
            _ = tick.tick() => { pending.retain(|_, request| !request.reply.is_closed() && request.started.elapsed() < REVEAL_TIMEOUT); }
            frame = socket.recv() => {
                // The upload a failure is about, so the browser drops that tab by id.
                let mut failed_tab = None;
                let result = match frame {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str::<ClientMessage>(&text) {
                        Ok(ClientMessage::Open { tab, name, size }) => { failed_tab = Some(tab); session.open(tab, name, size, bridge.config.max_upload_bytes).await }
                        Ok(ClientMessage::End { tab }) => { failed_tab = Some(tab); session.end(tab).await }
                        Ok(ClientMessage::State { tabs, active }) => session.state(tabs, active),
                        Ok(ClientMessage::Close { tab }) => { session.close(tab).await; Ok(()) },
                        Ok(ClientMessage::Reply { id, error }) => {
                            if let Some(request) = pending.remove(&id) { let _ = request.reply.send(error.map_or(Ok(()), |e| Err(anyhow::Error::msg(e)))); }
                            Ok(())
                        }
                        _ => Err("invalid bridge message".into()),
                    },
                    Some(Ok(Message::Binary(bytes))) => { failed_tab = session.uploading(); session.bytes(&bytes).await }
                    Some(Ok(Message::Ping(bytes))) => { if bounded(&mut socket, Message::Pong(bytes)).await.is_err() { break; } continue; },
                    Some(Ok(Message::Pong(_))) => continue,
                    _ => break,
                };
                if let Err(message) = result
                    && send(&mut socket, ServerMessage::Error { message, tab: failed_tab }).await.is_err() { break; }
                bridge.publish(session.snapshot.clone());
            }
        }
    }
    bridge.publish(Snapshot::default());
    for (_, request) in pending {
        let _ = request
            .reply
            .send(Err(anyhow::anyhow!("browser disconnected")));
    }
    while let Ok(command) = bridge
        .commands
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .try_recv()
    {
        match command {
            Command::Reveal { reply, .. } => {
                let _ = reply.send(Err(anyhow::anyhow!("browser disconnected")));
            }
            Command::OpenLink { reply, .. } => {
                let _ = reply.send(Err(anyhow::anyhow!("browser disconnected")));
            }
        }
    }
}

/// The browser app has no folders to search for the local copy of a figma.com link.
const NO_LOCAL_COPIES: &str =
    "the browser app cannot open local copies; open the .fig file in the browser tab first";
