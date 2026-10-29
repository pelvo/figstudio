//! `list_file_shaders` / `get_shader` with the file's shader folder (`FILE.fig.shaders/`):
//! sources as Figma's "View code" → "Download files" saves them, matched to the file's shaders.
use figstudio_mcp::FigstudioServer;
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/shaders.fig")
}

/// A `main.ts` declaring `names` and a `features.json` named `name`.
fn source(name: &str, names: &[&str]) -> Vec<(&'static str, String)> {
    let props: String = names
        .iter()
        .map(|n| format!("    {n}: {{ type: \"number\", label: \"{n}\", defaultValue: 0 }},\n"))
        .collect();
    vec![
        (
            "features.json",
            format!("{{\"name\": \"{name}\", \"version\": 2, \"isAnimated\": false}}"),
        ),
        (
            "main.ts",
            format!(
                "import {{ defineProperties }} from \"figma:shaders\";\nexport default function Effect() {{ }}\nexport function setup(device, frame) {{ var s = {{ a: 1 }}; }}\ndefineProperties(Effect, {{\n{props}}});\n"
            ),
        ),
    ]
}

fn zip(path: &Path, files: &[(&str, String)]) {
    let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, text) in files {
        z.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(text.as_bytes()).unwrap();
    }
    z.finish().unwrap();
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    args: Value,
) -> Value {
    client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
        .structured_content
        .unwrap()
}

#[tokio::test]
async fn downloaded_sources_give_shaders_their_name_and_code() {
    let dir = tempfile::tempdir().unwrap();
    let fig = dir.path().join("playground.fig");
    std::fs::copy(fixture(), &fig).unwrap();
    let shaders = dir.path().join("playground.fig.shaders");
    std::fs::create_dir(&shaders).unwrap();
    // Matched by its parameter names (the zip's name is Figma's, not the shader's ID).
    zip(
        &shaders.join("Concentric-patterns-source.zip"),
        &source(
            "Concentric patterns",
            &[
                "shape",
                "ringCount",
                "falloff",
                "inverseFalloff",
                "color",
                "offset",
                "transform",
            ],
        ),
    );
    // Matched by its folder's name: the shader's key.
    let by_key = shaders.join("33d4bf82a8f949caa6b12336d86f0b723ee8209e");
    std::fs::create_dir(&by_key).unwrap();
    for (name, text) in source("Pixelate", &["nothing", "alike"]) {
        std::fs::write(by_key.join(name), text).unwrap();
    }
    let (a, b) = tokio::io::duplex(65536);
    let server = tokio::spawn(FigstudioServer::new(vec![fig.clone()], true).serve(a));
    let client = ().serve(b).await.unwrap();
    let server = server.await.unwrap().unwrap();

    let list = call(&client, "list_file_shaders", json!({})).await;
    let shader = |id: &str| {
        list["shaders"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["nodeId"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(shader("1:5")["name"], "Concentric patterns");
    assert_eq!(shader("1:9")["name"], "Pixelate");
    assert!(shader("1:7")["name"].is_null());
    assert!(
        list["note"].as_str().unwrap().contains("Download files"),
        "{list}"
    );
    let files = shader("1:5")["files"].clone();
    assert_eq!(files.as_array().unwrap().len(), 2);
    assert!(
        files[0].get("content").is_none(),
        "the list leaves contents out"
    );

    let one = call(&client, "get_shader", json!({"id": "1:5"})).await;
    assert_eq!(one["name"], "Concentric patterns");
    assert_eq!(one["sourceVersion"], 2);
    let main = one["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["filename"] == "main.ts")
        .unwrap();
    assert!(
        main["content"]
            .as_str()
            .unwrap()
            .contains("defineProperties(Effect")
    );
    server.cancel().await.unwrap();
}
