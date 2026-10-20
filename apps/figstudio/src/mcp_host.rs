//! Agent access: the app hosts the figstudio MCP server (live mode) on loopback.
//!
//! Off by default; the saved Agent access toggle starts it. The server runs on its own thread
//! with a 2-worker tokio runtime, so the UI thread never waits on it: the snapshot it reads
//! is swapped under a lock held only for an `Arc` store, and its `reveal_node` requests come
//! back over a channel drained once per frame.
use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, PoisonError, RwLock};

use egui::RichText;
use figstudio_mcp::access::HttpAccess;
use figstudio_mcp::live::{Command, LiveFiles, Snapshot, TabInfo};
use figstudio_mcp::tools::design::Assets;
use figstudio_mcp::{FigstudioServer, bind_first, default_http_addrs, serve_http_with};
use tokio_util::sync::CancellationToken;

use crate::mcp_state::{Key, key, reveal, viewport};
use crate::tab::Tab;
use crate::theme;

/// The server thread's news: the bound address, or why it could not start or stopped.
type Event = anyhow::Result<SocketAddr>;

pub struct McpHost {
    /// Requested access, restored from preferences; a runtime failure never changes it.
    pub on: bool,
    /// Tried in order; the first free one is used (127.0.0.1:3845, then 3846 to 3855).
    addrs: Vec<SocketAddr>,
    /// "Allow remote agents": listen on `remote_addr` with a per-start bearer token.
    pub remote: bool,
    /// A specific address of this computer, for example its Tailscale IP with a port.
    pub remote_addr: String,
    /// A start-up problem found in `sync`, returned as a toast by the next `frame`.
    problem: Option<String>,
    /// A failed start stays stopped until off/on or the next launch, without losing intent.
    failed: bool,
    running: Option<Running>,
}

impl Default for McpHost {
    fn default() -> Self {
        Self {
            on: false,
            addrs: default_http_addrs(),
            remote: false,
            remote_addr: String::new(),
            problem: None,
            failed: false,
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
    /// The bearer token of a remote bind, for the Copy button.
    token: Option<String>,
    /// `reveal_node` links whose file no tab shows, for `App::go_to_link` (`link_open.rs`).
    links: Vec<crate::link_open::LinkAsk>,
}

impl McpHost {
    /// Browser-only (a linked-room proposal); the native server has no links to confirm.
    pub fn confirm(&mut self, _ctx: &egui::Context) {}

    /// The switches and the address, for Settings → Agents to draw as its rows.
    pub fn access(&mut self) -> Option<crate::settings::Access<'_>> {
        let endpoint = self.running.as_ref().map(|r| {
            let address = r
                .addr
                .map_or_else(|| "Starting…".to_owned(), |a| format!("{a}/mcp"));
            (address, r.token.clone())
        });
        Some(crate::settings::Access {
            on: &mut self.on,
            remote: &mut self.remote,
            remote_addr: &mut self.remote_addr,
            endpoint,
        })
    }

    pub fn menu(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.on, "Agent access (MCP server)")
            .on_hover_text(
                "Lets MCP clients read the open tabs and select nodes. Loopback only \
             unless 'Allow remote agents' is on. Remembered between launches.",
            );
        ui.add_enabled_ui(!self.on, |ui| {
            ui.checkbox(&mut self.remote, "Allow remote agents")
                .on_hover_text(
                    "Listen on the address below instead of loopback. Clients must send the \
                 bearer token shown in the status bar. Plain HTTP: use Tailscale or an SSH tunnel.",
                );
            if self.remote {
                ui.add(
                    egui::TextEdit::singleline(&mut self.remote_addr).hint_text("100.x.y.z:3845"),
                );
            }
        });
    }

    /// Where to listen and who may connect, from the window settings.
    fn plan(&self) -> Result<(Vec<SocketAddr>, HttpAccess), String> {
        if !self.remote {
            return Ok((self.addrs.clone(), HttpAccess::loopback()));
        }
        let addr: SocketAddr = self.remote_addr.trim().parse().map_err(|_| {
            "Enter this computer's address and port, like 100.x.y.z:3845".to_owned()
        })?;
        if addr.ip().is_unspecified() {
            return Err("Enter one specific address, not 0.0.0.0 or ::".to_owned());
        }
        let access = HttpAccess::for_bind(addr, &[], false).map_err(|e| e.to_string())?;
        Ok((vec![addr], access))
    }

    pub fn status(&self, ui: &mut egui::Ui) {
        let Some(running) = &self.running else { return };
        let text = running
            .addr
            .map_or_else(|| "MCP starting…".to_owned(), |a| format!("MCP {a}/mcp"));
        ui.label(RichText::new(text).color(theme::of(ui).text_dim))
            .on_hover_text("Point your MCP client here.");
        if let Some(token) = &running.token
            && ui
                .small_button("Copy token")
                .on_hover_text("Send it as 'Authorization: Bearer <token>'.")
                .clicked()
        {
            ui.ctx().copy_text(token.clone());
        }
    }

    /// Start of each frame: answer the server's requests (`reveal_node`). Returns an error
    /// to show as a toast when the server could not start or stopped.
    pub fn frame(&mut self, tabs: &mut [Tab], active: &mut usize) -> Option<String> {
        if let Some(problem) = self.problem.take() {
            return Some(problem);
        }
        let running = self.running.as_mut()?;
        if let Err(error) = running.poll_events() {
            self.stop();
            self.failed = true;
            return Some(figstudio_fig::one_line(error.as_ref()));
        }
        running.drain(tabs, active);
        None
    }

    /// The figma.com links `reveal_node` asked the app to open since the last call.
    pub fn take_links(&mut self) -> Vec<crate::link_open::LinkAsk> {
        self.running
            .as_mut()
            .map(|running| std::mem::take(&mut running.links))
            .unwrap_or_default()
    }

    /// End of each frame: start or stop as the toggle says, and publish the tabs when what
    /// the tools see changed.
    pub fn sync(&mut self, ctx: &egui::Context, tabs: &[Tab], active: usize) {
        match (self.on, self.running.is_some(), self.failed) {
            (true, false, false) => match self.plan() {
                Ok((addrs, access)) => self.running = Some(start(ctx, addrs, access)),
                Err(problem) => {
                    self.failed = true;
                    self.problem = Some(problem);
                }
            },
            (false, _, _) => {
                self.failed = false;
                self.stop();
            }
            _ => {}
        }
        if let Some(running) = self.running.as_mut() {
            running.sync(tabs, active);
        }
    }

    /// [`Self::sync`], then send the link answers: an MCP caller told a file is open finds
    /// it in the published snapshot.
    pub fn sync_then_answer(
        &mut self,
        ctx: &egui::Context,
        tabs: &[Tab],
        active: usize,
        answers: Vec<crate::link_open::Answer>,
    ) {
        self.sync(ctx, tabs, active);
        for (done, result) in answers {
            done(result);
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

    /// Answer `reveal_node`: make the tab active, select the node and fit it into view. Links
    /// to open are kept for [`McpHost::take_links`].
    fn drain(&mut self, tabs: &mut [Tab], active: &mut usize) {
        while let Ok(command) = self.commands.try_recv() {
            match command {
                Command::Reveal { tab, node, reply } => {
                    let result = reveal(tabs, active, tab, node).map_err(anyhow::Error::msg);
                    // The tool may have timed out and dropped its receiver; nothing to tell then.
                    let _ = reply.send(result);
                }
                Command::OpenLink { link, asked, reply } => {
                    let done: crate::link_open::Done = Box::new(move |result| {
                        let _ = reply.send(result.map_err(anyhow::Error::msg));
                    });
                    self.links
                        .push(crate::link_open::LinkAsk { link, asked, done });
                }
            }
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

fn start(ctx: &egui::Context, addrs: Vec<SocketAddr>, access: HttpAccess) -> Running {
    let snapshot = Arc::new(RwLock::new(Arc::new(Snapshot::default())));
    let (commands, inbox) = mpsc::channel();
    let (events, news) = mpsc::channel();
    let cancel = CancellationToken::new();
    let wake_ctx = ctx.clone();
    let wake = Arc::new(move || wake_ctx.request_repaint());
    let files = Arc::new(LiveFiles::new(snapshot.clone(), commands, wake).with_link_opening());
    let bearer = access.token().map(str::to_owned);
    let (token, ctx, failed) = (cancel.clone(), ctx.clone(), events.clone());
    let spawned = std::thread::Builder::new()
        .name("mcp".into())
        .spawn(move || {
            if let Err(error) = serve(addrs, files, access, token, &events, &ctx) {
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
        token: bearer,
        links: Vec::new(),
    }
}

/// The server thread: bind, report the address, serve until cancelled.
fn serve(
    addrs: Vec<SocketAddr>,
    files: Arc<LiveFiles>,
    access: HttpAccess,
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
        serve_http_with(listener, server, cancel, access)
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
