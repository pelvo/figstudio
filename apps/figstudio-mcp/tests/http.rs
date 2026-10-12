use figstudio_mcp::{FigstudioServer, serve_http};
use futures_util::StreamExt;
use rmcp::{
    model::{ClientJsonRpcMessage, ProtocolVersion},
    transport::streamable_http_client::{StreamableHttpClient, StreamableHttpPostResponse},
};
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio_util::sync::CancellationToken;

// Use rmcp's HTTP and SSE decoder; do not implement a second transport in the test.
async fn post(
    client: &reqwest::Client,
    uri: &Arc<str>,
    version: &str,
    session: Option<Arc<str>>,
    body: Value,
) -> (Value, Option<String>) {
    let mut headers = HashMap::new();
    headers.insert(
        reqwest::header::HeaderName::from_static("mcp-protocol-version"),
        version.parse().unwrap(),
    );
    if version == ProtocolVersion::LATEST.as_str() {
        headers.insert(
            reqwest::header::HeaderName::from_static("mcp-method"),
            body["method"].as_str().unwrap().parse().unwrap(),
        );
        if let Some(name) = body["params"]["name"].as_str() {
            headers.insert(
                reqwest::header::HeaderName::from_static("mcp-name"),
                name.parse().unwrap(),
            );
        }
    }
    let message: ClientJsonRpcMessage = serde_json::from_value(body).unwrap();
    match client
        .post_message(uri.clone(), message, session, None, headers)
        .await
        .unwrap()
    {
        StreamableHttpPostResponse::Json(response, session) => {
            (serde_json::to_value(response).unwrap(), session)
        }
        StreamableHttpPostResponse::Sse(mut stream, session) => {
            while let Some(event) = stream.next().await {
                let data = event.unwrap().data.unwrap_or_default();
                if data.trim().is_empty() {
                    continue;
                }
                let response: Value = serde_json::from_str(&data).unwrap();
                if response.get("id").is_some() {
                    return (response, session);
                }
            }
            panic!("SSE response ended without a JSON-RPC reply");
        }
        other => panic!("expected JSON-RPC reply, got {other:?}"),
    }
}

#[tokio::test]
async fn http_march_2025() {
    check_version("2025-03-26").await;
}
#[tokio::test]
async fn http_june_2025() {
    check_version("2025-06-18").await;
}
#[tokio::test]
async fn http_november_2025() {
    check_version("2025-11-25").await;
}
#[tokio::test]
async fn http_latest_stateless() {
    check_version(ProtocolVersion::LATEST.as_str()).await;
}

async fn check_version(version: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri: Arc<str> = format!("http://{}/mcp", listener.local_addr().unwrap()).into();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(serve_http(listener, FigstudioServer::new(vec![fixture], true), shutdown.clone()));
        let client = reqwest::Client::new();
        let version = version.to_owned();
        {
            let (reply, session) = post(&client, &uri, &version, None, json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
                "protocolVersion":version, "capabilities":{}, "clientInfo":{"name":"test", "version":"1"}
            }})).await;
            let negotiated = if version == ProtocolVersion::LATEST.to_string() {
                ProtocolVersion::LATEST_WITH_INITIALIZE.to_string()
            } else { version.clone() };
            assert_eq!(reply["result"]["protocolVersion"], negotiated);
            assert_eq!(reply["result"]["serverInfo"]["name"], "figstudio-mcp");
            // initialize falls back to a session-based version, including for LATEST.
            assert!(session.is_some());
            // Latest ordinary requests use per-request metadata, not the fallback session.
            let session: Option<Arc<str>> = if version == ProtocolVersion::LATEST.to_string() { None } else { session.map(Into::into) };
            if let Some(session) = &session {
                let response = client.post(uri.as_ref()).header("Accept", "application/json, text/event-stream").header("MCP-Protocol-Version", &version)
                    .header("Mcp-Session-Id", session.as_ref()).json(&json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))
                    .send().await.unwrap();
                assert!(response.status().is_success(), "initialized notification rejected: {}", response.status());
            }
            let meta = json!({"io.modelcontextprotocol/protocolVersion":version, "io.modelcontextprotocol/clientInfo":{"name":"test", "version":"1"}, "io.modelcontextprotocol/clientCapabilities":{}});
            let (tools, response_session) = post(&client, &uri, &version, session.clone(), json!({"jsonrpc":"2.0", "id":2, "method":"tools/list", "params":{"_meta":meta}})).await;
            if version == ProtocolVersion::LATEST.to_string() { assert!(response_session.is_none()); }
            let tools = tools["result"]["tools"].as_array().unwrap_or_else(|| panic!("tools/list failed: {tools}"));
            assert_eq!(tools.len(), 14);
            assert!(tools.iter().any(|tool| tool["name"] == "get_design_context"));
            assert!(tools.iter().any(|tool| tool["name"] == "get_variable_defs"));
            assert!(tools.iter().any(|tool| tool["name"] == "download_assets"));
            assert!(tools.iter().any(|tool| tool["name"] == "get_figjam"));
            assert!(tools.iter().any(|tool| tool["name"] == "get_metadata"));
            assert!(tools.iter().any(|tool| tool["name"] == "get_screenshot"));
            assert!(tools.iter().any(|tool| tool["name"] == "whoami"));
            assert!(tools.iter().any(|tool| tool["name"] == "list_files"));
            for name in ["get_context_for_code_connect", "get_code_connect_map", "add_code_connect_map", "search_design_system", "get_libraries"] {
                assert!(tools.iter().any(|tool| tool["name"] == name));
            }
            let (whoami, _) = post(&client, &uri, &version, session.clone(), json!({"jsonrpc":"2.0", "id":3, "method":"tools/call", "params":{"name":"whoami", "arguments":{}, "_meta":meta}})).await;
            assert_eq!(whoami["result"]["structuredContent"], json!({"mode":"offline", "server":"figstudio-mcp", "files":["structure"]}), "whoami failed: {whoami}");
            assert_ne!(whoami["result"]["isError"], true);
            let (unknown, _) = post(&client, &uri, &version, session.clone(), json!({"jsonrpc":"2.0", "id":4, "method":"tools/call", "params":{"name":"unknown", "arguments":{}, "_meta":meta}})).await;
            assert!(unknown.get("error").is_some());
        }
        let denied = client.post(uri.as_ref()).header("Host", "example.invalid")
            .json(&json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{"protocolVersion":"2025-03-26", "capabilities":{}, "clientInfo":{"name":"test", "version":"1"}}}))
            .send().await.unwrap();
        assert_eq!(denied.status(), reqwest::StatusCode::FORBIDDEN);
        assert!(denied.headers().get("access-control-allow-origin").is_none());
        // Origin: absent or loopback passes; a web origin or `null` is rejected.
        let init = json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{"protocolVersion":"2025-03-26", "capabilities":{}, "clientInfo":{"name":"test", "version":"1"}}});
        let origin_status = |origin: Option<&'static str>| {
            let client = &client;
            let uri = &uri;
            let init = &init;
            async move {
                let mut request = client.post(uri.as_ref()).header("Accept", "application/json, text/event-stream").json(init);
                if let Some(origin) = origin {
                    request = request.header("Origin", origin);
                }
                let response = request.send().await.unwrap();
                // No CORS grant: a browser page must never read the reply, whatever the status.
                assert!(response.headers().get("access-control-allow-origin").is_none(), "CORS header for {origin:?}");
                response.status()
            }
        };
        assert_eq!(origin_status(None).await, reqwest::StatusCode::OK);
        assert_eq!(origin_status(Some("http://localhost:5173")).await, reqwest::StatusCode::OK);
        assert_eq!(origin_status(Some("http://127.0.0.1:3000")).await, reqwest::StatusCode::OK);
        assert_eq!(origin_status(Some("https://evil.example")).await, reqwest::StatusCode::FORBIDDEN);
        assert_eq!(origin_status(Some("null")).await, reqwest::StatusCode::FORBIDDEN);
        let preflight = client.request(reqwest::Method::OPTIONS, uri.as_ref()).header("Origin", "https://evil.example")
            .header("Access-Control-Request-Method", "POST").send().await.unwrap();
        assert!(preflight.headers().get("access-control-allow-origin").is_none(), "preflight granted CORS");
        shutdown.cancel();
        task.await.unwrap().unwrap();
    }).await.expect("HTTP protocol timed out");
}
