//! `get_prototype` through the protocol, on a prototype built from the public fixture.
#[path = "../../../crates/render/tests/common/prototype.rs"]
mod proto;

use figstudio_mcp::FigstudioServer;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
};
use serde_json::{Value, json};

type Client = rmcp::service::RunningService<rmcp::RoleClient, ()>;

async fn call(client: &Client, args: Value) -> CallToolResult {
    let request = CallToolRequestParams::new("get_prototype")
        .with_arguments(args.as_object().unwrap().clone());
    client.call_tool(request).await.unwrap()
}

async fn run(check: impl AsyncFnOnce(&Client)) {
    let dir = tempfile::tempdir().unwrap();
    let paths = vec![proto::prototype_file(dir.path())];
    let (client_io, server_io) = tokio::io::duplex(1 << 22);
    let task = tokio::spawn(async move {
        FigstudioServer::new(paths, true)
            .serve(server_io)
            .await
            .unwrap()
            .waiting()
            .await
    });
    let client = ().serve(client_io).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(60), check(&client))
        .await
        .expect("timed out");
    client.cancel().await.unwrap();
    task.await.unwrap().unwrap();
}

fn structured(result: &CallToolResult) -> Value {
    assert_ne!(result.is_error, Some(true), "{result:?}");
    result.structured_content.clone().unwrap()
}

#[tokio::test]
async fn lists_flows_screens_and_links() {
    run(async |client| {
        let v = structured(&call(client, json!({})).await);
        assert_eq!(
            v["flows"],
            json!([{"name": "Start", "description": "", "page": "0:1", "startNodeId": "1:2"}])
        );
        let screens: Vec<_> = v["screens"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["nodeId"].clone())
            .collect();
        assert_eq!(screens, [json!("1:2"), json!("1:4")]);
        let link = &v["screens"][0]["links"][0];
        assert_eq!(
            (
                &link["nodeId"],
                &link["trigger"],
                &link["action"],
                &link["destinationId"],
                &link["played"]
            ),
            (
                &json!("1:53"),
                &json!("ON_CLICK"),
                &json!("NAVIGATE"),
                &json!("1:4"),
                &json!(true)
            )
        );
        assert_eq!(link["rect"].as_array().unwrap().len(), 4);
        assert_eq!(
            (&v["notPlayed"], &v["truncated"]),
            (&json!(1), &json!(false))
        );

        let second = structured(&call(client, json!({"nodeId": "1:4"})).await);
        assert_eq!(
            second["screens"].as_array().unwrap().len(),
            1,
            "back adds no screen"
        );
        assert_eq!(second["truncated"], json!(false));
        let cut = structured(&call(client, json!({"nodeId": "1:2", "limit": 1})).await);
        assert_eq!(cut["screens"].as_array().unwrap().len(), 1);
        assert_eq!(cut["truncated"], json!(true), "1:2 links on to 1:4");
    })
    .await;
}

#[tokio::test]
async fn bad_node_ids_are_tool_errors() {
    run(async |client| {
        for (args, message) in [
            (json!({"nodeId": "9:9"}), "does not exist"),
            (
                json!({"nodeId": "I1:38;1:1"}),
                "not a node inside an instance",
            ),
            (json!({"nodeId": "nope"}), "node"),
        ] {
            let result = call(client, args).await;
            assert_eq!(result.is_error, Some(true));
            assert!(format!("{result:?}").contains(message), "{result:?}");
        }
    })
    .await;
}
