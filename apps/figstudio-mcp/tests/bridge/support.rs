pub use figstudio_mcp::bridge::{BridgeConfig, serve_bridge};
pub use figstudio_mcp_core::bridge::{ClientMessage, PROTOCOL_VERSION, ServerMessage};
pub use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
pub use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};
pub use tokio_util::sync::CancellationToken;
pub type Socket = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub struct BridgeTest {
    pub addr: std::net::SocketAddr,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl BridgeTest {
    pub async fn start(cap: u64) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = CancellationToken::new();
        let config = BridgeConfig {
            token: "test-pairing-secret".into(),
            origins: vec!["http://localhost:8765".into()],
            max_upload_bytes: cap,
        };
        let task = tokio::spawn(serve_bridge(
            listener,
            config,
            true,
            24000,
            Vec::new(),
            stop.clone(),
            figstudio_mcp::access::HttpAccess::loopback(),
        ));
        Self { addr, stop, task }
    }
    /// A paired-origin socket that has not said hello yet.
    pub async fn connect(&self) -> Socket {
        let mut request = format!("ws://{}/bridge", self.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Origin", "http://localhost:8765".parse().unwrap());
        tokio_tungstenite::connect_async(request).await.unwrap().0
    }
    pub async fn socket(&self, token: &str) -> Socket {
        let mut socket = self.connect().await;
        send(
            &mut socket,
            ClientMessage::Hello {
                token: token.into(),
                version: PROTOCOL_VERSION,
            },
        )
        .await;
        socket
    }
    pub async fn finish(self) {
        self.stop.cancel();
        self.task.await.unwrap().unwrap();
    }
}
pub async fn send(socket: &mut Socket, message: ClientMessage) {
    socket
        .send(Message::Text(
            serde_json::to_string(&message).unwrap().into(),
        ))
        .await
        .unwrap();
}
pub async fn receive(socket: &mut Socket) -> ServerMessage {
    let frame = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(frame.to_text().unwrap()).unwrap()
}
pub fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/structure.fig"
    ))
    .unwrap()
}
pub async fn upload(socket: &mut Socket) {
    let bytes = fixture();
    send(
        socket,
        ClientMessage::Open {
            tab: 7,
            name: "structure.fig".into(),
            size: bytes.len() as u64,
        },
    )
    .await;
    for chunk in bytes.chunks(64 * 1024) {
        socket
            .send(Message::Binary(chunk.to_vec().into()))
            .await
            .unwrap();
    }
    send(socket, ClientMessage::End { tab: 7 }).await;
    // State is serialized after decode; this invalid request returns an ordering barrier.
    send(socket, ClientMessage::End { tab: 99 }).await;
    assert!(matches!(receive(socket).await, ServerMessage::Error { .. }));
}
