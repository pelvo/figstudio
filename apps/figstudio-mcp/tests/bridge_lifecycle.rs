#[path = "bridge/support.rs"]
mod support;
use support::*;

#[tokio::test]
async fn replacement_version_and_declared_size_are_enforced() {
    let bridge = BridgeTest::start(2 * 1024 * 1024).await;
    let mut old = bridge.socket("test-pairing-secret").await;
    assert!(matches!(
        receive(&mut old).await,
        ServerMessage::Welcome { .. }
    ));
    upload(&mut old).await;
    let mut replacement = bridge.socket("test-pairing-secret").await;
    assert!(matches!(
        receive(&mut replacement).await,
        ServerMessage::Welcome { .. }
    ));
    assert!(matches!(
        old.next().await.unwrap().unwrap(),
        Message::Close(_)
    ));
    send(
        &mut replacement,
        ClientMessage::Open {
            tab: 1,
            name: "bad.fig".into(),
            size: 1,
        },
    )
    .await;
    replacement
        .send(Message::Binary(vec![1, 2].into()))
        .await
        .unwrap();
    assert!(
        matches!(receive(&mut replacement).await, ServerMessage::Error { message, .. } if message.contains("declared"))
    );
    replacement.close(None).await.unwrap();
    let mut request = format!("ws://{}/bridge", bridge.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Origin", "http://localhost:8765".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    send(
        &mut socket,
        ClientMessage::Hello {
            token: "test-pairing-secret".into(),
            version: PROTOCOL_VERSION + 1,
        },
    )
    .await;
    assert!(
        matches!(socket.next().await.unwrap().unwrap(), Message::Close(Some(frame)) if u16::from(frame.code) == 4426)
    );
    bridge.finish().await;
}

#[tokio::test]
async fn a_socket_that_never_says_hello_is_closed() {
    let bridge = BridgeTest::start(1024).await;
    let mut silent = bridge.connect().await;
    let frame = tokio::time::timeout(std::time::Duration::from_secs(10), silent.next())
        .await
        .expect("closed within the hello timeout");
    assert!(
        matches!(frame, Some(Ok(Message::Close(Some(frame)))) if u16::from(frame.code) == 4401)
    );
    bridge.finish().await;
}

#[tokio::test]
async fn a_failed_decode_names_its_tab() {
    let bridge = BridgeTest::start(1024).await;
    let mut socket = bridge.socket("test-pairing-secret").await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Welcome { .. }
    ));
    let junk = b"not a fig file";
    let open = ClientMessage::Open {
        tab: 3,
        name: "junk.fig".into(),
        size: junk.len() as u64,
    };
    send(&mut socket, open).await;
    socket
        .send(Message::Binary(junk.to_vec().into()))
        .await
        .unwrap();
    send(&mut socket, ClientMessage::End { tab: 3 }).await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Error { tab: Some(3), .. }
    ));
    // Errors that are not about an upload name no tab.
    socket.send(Message::Text("{}".into())).await.unwrap();
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Error { tab: None, .. }
    ));
    bridge.finish().await;
}

#[tokio::test]
async fn a_peer_that_stops_reading_does_not_block_the_next_pairing() {
    let bridge = BridgeTest::start(1024).await;
    let mut stalled = bridge.socket("test-pairing-secret").await;
    assert!(matches!(
        receive(&mut stalled).await,
        ServerMessage::Welcome { .. }
    ));
    // Each bad frame earns a longer error reply. Never reading them fills both directions
    // until our own send stalls: the bridge is then blocked writing to us.
    let mut stalled_send = false;
    for _ in 0..5_000_000 {
        let frame = stalled.send(Message::Text("x".into()));
        match tokio::time::timeout(std::time::Duration::from_millis(500), frame).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => break,
            Err(_) => {
                stalled_send = true;
                break;
            }
        }
    }
    assert!(stalled_send, "the flood never filled the socket buffers");
    let mut next = bridge.socket("test-pairing-secret").await;
    assert!(matches!(
        receive(&mut next).await,
        ServerMessage::Welcome { .. }
    ));
    drop(stalled);
    bridge.finish().await;
}
