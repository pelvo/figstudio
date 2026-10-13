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
        matches!(receive(&mut replacement).await, ServerMessage::Error { message } if message.contains("declared"))
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
