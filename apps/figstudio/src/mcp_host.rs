//! Agent access: the app hosts the figstudio MCP server (live mode) on loopback.
//!
//! Off at every launch; the View menu toggle starts it. The server runs on its own thread
//! with a 2-worker tokio runtime, so the UI thread never waits on it: the snapshot it reads
//! is swapped under a lock held only for an `Arc` store, and its `reveal_node` requests come
//! back over a channel drained once per frame.
use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, PoisonError, RwLock};

use egui::RichText;
use figstudio_mcp::live::{Command, LiveFiles, Snapshot, TabInfo};
use figstudio_mcp::tools::design::Assets;
use figstudio_mcp::{FigstudioServer, bind_first, default_http_addrs, serve_http};
use tokio_util::sync::CancellationToken;

use crate::mcp_state::{Key, key, reveal, viewport};
use crate::tab::Tab;
use crate::theme;

/// The server thread's news: the bound address, or why it could not start or stopped.
type Event = anyhow::Result<SocketAddr>;

pub struct McpHost {
    /// The View menu toggle. Never saved: every launch starts off.
    pub on: bool,
    /// Tried in order; the first free one is used (127.0.0.1:3845, then 3846 to 3855).
    addrs: Vec<SocketAddr>,
    running: Option<Running>,
}

impl Default for McpHost {
    fn default() -> Self {
        Self {
            on: false,
            addrs: default_http_addrs(),
            running: None,
        }
    }
}

struct Running {
    /// `None` until the server thread has bound its port.
    addr: Option<SocketAddr>,
    cancel: CancellationToken,
    snapshot: Arc<RwLock<Arc<Snapshot>>>,
    commands: Receiver<Command>,
    events: Receiver<Event>,
    /// What the published snapshot was built from.
    last_key: Option<Key>,
}

impl McpHost {
    pub fn menu(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.on, "Agent access (MCP server)")
            .on_hover_text(
                "Lets MCP clients on this computer read the open tabs and select nodes. \
             Loopback only; off at every launch.",
            );
    }

    pub fn status(&self, ui: &mut egui::Ui) {
        let Some(running) = &self.running else { return };
        let text = running
            .addr
            .map_or_else(|| "MCP starting…".to_owned(), |a| format!("MCP {a}/mcp"));
        ui.label(RichText::new(text).color(theme::TEXT_DIM))
            .on_hover_text("Loopback only. Point your MCP client here.");
        ui.separator();
    }

    /// Start of each frame: answer the server's requests (`reveal_node`). Returns an error
    /// to show as a toast when the server could not start or stopped.
    pub fn frame(&mut self, tabs: &mut [Tab], active: &mut usize) -> Option<String> {
        let running = self.running.as_mut()?;
        if let Err(error) = running.poll_events() {
            self.stop();
            self.on = false;
            return Some(figstudio_fig::one_line(error.as_ref()));
        }
        running.drain(tabs, active);
        None
    }

    /// End of each frame: start or stop as the toggle says, and publish the tabs when what
    /// the tools see changed.
    pub fn sync(&mut self, ctx: &egui::Context, tabs: &[Tab], active: usize) {
        match (self.on, self.running.is_some()) {
            (true, false) => self.running = Some(start(ctx, self.addrs.clone())),
            (false, true) => self.stop(),
            _ => {}
        }
        if let Some(running) = self.running.as_mut() {
            running.sync(tabs, active);
        }
    }

    /// Cancel the server; its thread winds down on its own (never joined on the UI thread).
    pub fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancel.cancel();
        }
    }

    #[cfg(test)]
    fn addr(&self) -> Option<SocketAddr> {
        self.running.as_ref().and_then(|r| r.addr)
    }
}

impl Running {
    fn poll_events(&mut self) -> anyhow::Result<()> {
        loop {
            match self.events.try_recv() {
                Ok(Ok(addr)) => self.addr = Some(addr),
                Ok(Err(error)) => return Err(error),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(()),
            }
        }
    }

    /// Answer `reveal_node`: make the tab active, select the node and fit it into view.
    fn drain(&mut self, tabs: &mut [Tab], active: &mut usize) {
        while let Ok(Command::Reveal { tab, node, reply }) = self.commands.try_recv() {
            let result = reveal(tabs, active, tab, node).map_err(anyhow::Error::msg);
            // The tool may have timed out and dropped its receiver; nothing to tell then.
            let _ = reply.send(result);
        }
    }

    fn sync(&mut self, tabs: &[Tab], active: usize) {
        let key = key(tabs, active);
        if self.last_key.as_ref() == Some(&key) {
            return;
        }
        let snapshot = Arc::new(snapshot(tabs, active));
        *self
            .snapshot
            .write()
            .unwrap_or_else(PoisonError::into_inner) = snapshot;
        self.last_key = Some(key);
    }
}

fn snapshot(tabs: &[Tab], active: usize) -> Snapshot {
    Snapshot {
        tabs: tabs
            .iter()
            .map(|t| TabInfo {
                id: t.id,
                path: t.path.clone(),
                doc: t.doc.clone(),
                page: t.page,
                selection: t.selection,
                viewport: viewport(t),
            })
            .collect(),
        active: tabs.get(active).map(|t| t.id),
    }
}

fn start(ctx: &egui::Context, addrs: Vec<SocketAddr>) -> Running {
    let snapshot = Arc::new(RwLock::new(Arc::new(Snapshot::default())));
    let (commands, inbox) = mpsc::channel();
    let (events, news) = mpsc::channel();
    let cancel = CancellationToken::new();
    let wake_ctx = ctx.clone();
    let wake = Arc::new(move || wake_ctx.request_repaint());
    let files = Arc::new(LiveFiles::new(snapshot.clone(), commands, wake));
    let (token, ctx, failed) = (cancel.clone(), ctx.clone(), events.clone());
    let spawned = std::thread::Builder::new()
        .name("mcp".into())
        .spawn(move || {
            if let Err(error) = serve(addrs, files, token, &events, &ctx) {
                let _ = events.send(Err(error));
                ctx.request_repaint();
            }
        });
    if let Err(error) = spawned {
        let error = anyhow::Error::from(error).context("Could not start the MCP server");
        let _ = failed.send(Err(error));
    }
    Running {
        addr: None,
        cancel,
        snapshot,
        commands: inbox,
        events: news,
        last_key: None,
    }
}

/// The server thread: bind, report the address, serve until cancelled.
fn serve(
    addrs: Vec<SocketAddr>,
    files: Arc<LiveFiles>,
    cancel: CancellationToken,
    events: &Sender<Event>,
    ctx: &egui::Context,
) -> anyhow::Result<()> {
    use anyhow::Context;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("mcp-worker")
        .enable_all()
        .build()
        .context("Could not start the MCP server")?;
    let result = runtime.block_on(async {
        let listener = bind_first(addrs)
            .await
            .map_err(|e| anyhow::anyhow!("Could not start the MCP server: no free port ({e})"))?;
        let addr = listener
            .local_addr()
            .context("Could not start the MCP server")?;
        let _ = events.send(Ok(addr));
        ctx.request_repaint();
        // Inline assets: the app's working directory is arbitrary, so it writes no files.
        let server = FigstudioServer::live(files, false).with_assets(Assets::Inline);
        serve_http(listener, server, cancel)
            .await
            .context("The MCP server stopped")
    });
    // An open SSE stream must not keep this thread alive after cancel.
    runtime.shutdown_background();
    result
}

#[cfg(test)]
#[path = "mcp_host_tests.rs"]
mod tests;
