//! A fake app for live mode: a snapshot built from fixtures and a command loop.
use figstudio_fig::{Document, Guid};
use figstudio_mcp::{
    FigstudioServer,
    live::{Command, LiveFiles, Snapshot, TabInfo},
};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, RwLock, mpsc},
};

pub type Client = rmcp::service::RunningService<rmcp::RoleClient, ()>;

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

pub fn doc(name: &str) -> Arc<Document> {
    Arc::new(Document::open(fixture(name)).unwrap())
}

pub fn tab(id: u64, path: PathBuf, doc: Arc<Document>, selection: Option<Guid>) -> TabInfo {
    TabInfo {
        id,
        path,
        page: doc.pages[0],
        doc,
        selection,
        viewport: Some([0.0, 0.0, 800.0, 600.0]),
    }
}

/// How the fake app answers [`Command::Reveal`].
pub enum Answer {
    /// `Ok` when the tab is open, else `Err("tab closed")`.
    Reply,
    /// Keep the reply sender and never answer.
    Never,
}

pub struct App {
    pub snapshot: Arc<RwLock<Arc<Snapshot>>>,
    pub client: Client,
    /// Every `(tab, node)` the app was asked to reveal.
    pub revealed: Arc<std::sync::Mutex<Vec<(u64, Guid)>>>,
    pub wakes: Arc<std::sync::atomic::AtomicUsize>,
}

impl App {
    pub fn replace(&self, snapshot: Snapshot) {
        *self.snapshot.write().unwrap() = Arc::new(snapshot);
    }

    pub async fn call(&self, tool: &'static str, args: Value) -> CallToolResult {
        call(&self.client, tool, args).await
    }
}

pub async fn call(client: &Client, tool: &'static str, args: Value) -> CallToolResult {
    let request =
        CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone());
    client.call_tool(request).await.unwrap()
}

/// Serve `snapshot` in live mode over a duplex pipe, with a fake app loop on a thread.
pub async fn start(snapshot: Snapshot, answer: Answer) -> App {
    let snapshot = Arc::new(RwLock::new(Arc::new(snapshot)));
    let (commands, inbox) = mpsc::channel::<Command>();
    let revealed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = revealed.clone();
    let shared = snapshot.clone();
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        while let Ok(Command::Reveal { tab, node, reply }) = inbox.recv() {
            seen.lock().unwrap().push((tab, node));
            match answer {
                Answer::Reply => {
                    let open = shared.read().unwrap().tabs.iter().any(|t| t.id == tab);
                    let _ = reply.send(if open {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("tab closed"))
                    });
                }
                Answer::Never => kept.push(reply),
            }
        }
    });
    let counter = wakes.clone();
    let wake = Arc::new(move || {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let files = Arc::new(LiveFiles::new(snapshot.clone(), commands, wake));
    let server = FigstudioServer::live(files, true)
        .with_assets(figstudio_mcp::tools::design::Assets::Inline);
    let (client_io, server_io) = tokio::io::duplex(1 << 24);
    tokio::spawn(async move { server.serve(server_io).await.unwrap().waiting().await });
    let client = ().serve(client_io).await.unwrap();
    App {
        snapshot,
        client,
        revealed,
        wakes,
    }
}

pub fn texts(result: &CallToolResult) -> String {
    let value = serde_json::to_value(result).unwrap();
    value["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
