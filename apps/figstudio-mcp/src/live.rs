//! Live mode: the tools read the documents the app has open, through a published snapshot.
//!
//! The app owns the documents and publishes an [`Arc<Snapshot>`] whenever its tabs, pages or
//! selection change; a tool call clones the `Arc<Document>` it needs, so closing a tab never
//! invalidates a running call. Requests back to the app ([`Command`]) go over a channel the app
//! drains once per frame, after [`LiveFiles`] calls its `wake` hook.
use crate::{
    error::McpError,
    files::{FileInfo, IMAGE_CACHE_BYTES, find, stem},
    tools::rendering::Rendering,
};
use figstudio_fig::{Document, Guid};
use figstudio_render::images::Images;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError, RwLock, mpsc::Sender},
    time::Duration,
};
use tokio::sync::oneshot;

/// How long `reveal_node` waits for the app.
pub const REVEAL_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `reveal_node` waits for the app to open a local copy: a large file loads for seconds.
/// The whole request shares it: the app may list its link folders for up to 20 s first, and
/// answers "still opening" a few seconds before this runs out if the load has not finished
/// (the file still opens and shows the node afterwards).
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(60);

/// What the app shows, as the tools see it.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Open tabs in tab order.
    pub tabs: Vec<TabInfo>,
    /// [`TabInfo::id`] of the active tab.
    pub active: Option<u64>,
}

/// One open tab.
#[derive(Clone, Debug)]
pub struct TabInfo {
    /// Stable while the tab is open; never reused for another tab.
    pub id: u64,
    /// The file on disk, read for lazily loaded images.
    pub path: PathBuf,
    /// The app's document; tools keep their own clone while they run.
    pub doc: Arc<Document>,
    /// The page (canvas) shown.
    pub page: Guid,
    /// The selected node.
    pub selection: Option<Guid>,
    /// The visible part of the page, `[x, y, w, h]` in page units.
    pub viewport: Option<[f64; 4]>,
}

/// A request to the app.
#[derive(Debug)]
pub enum Command {
    /// Make `tab` active, select `node` and fit it into view.
    Reveal {
        /// [`TabInfo::id`].
        tab: u64,
        /// A document node of that tab.
        node: Guid,
        /// `Ok`, or why it could not be done (such as "tab closed").
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    /// Show a figma.com link's node: in the one open tab or local copy that matches it (the
    /// app searches its link folders; a tie is an error), opening the copy first. Only sent
    /// when [`LiveFiles::opens_links`].
    OpenLink {
        /// The link as the tool was given it.
        link: String,
        /// What was done, one line, or why it could not be done.
        reply: oneshot::Sender<anyhow::Result<String>>,
    },
}

/// The app side of live mode, shared by every MCP session.
pub struct LiveFiles {
    snapshot: Arc<RwLock<Arc<Snapshot>>>,
    commands: Sender<Command>,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// The app finds figma.com links itself ([`Command::OpenLink`]); the browser bridge cannot.
    opens_links: bool,
    /// Per [`Document::instance`]: its own image cache, never the app's (shared in-flight
    /// image requests could hang when the app closes a document).
    rendering: Mutex<HashMap<u64, Rendering>>,
}

impl std::fmt::Debug for LiveFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveFiles")
            .field("tabs", &self.current().tabs.len())
            .finish_non_exhaustive()
    }
}

/// A tab picked for a call, with its file key. Holds the snapshot, not a document clone.
#[derive(Clone, Debug)]
pub(crate) struct Picked {
    pub key: String,
    snapshot: Arc<Snapshot>,
    index: usize,
}

impl Picked {
    pub fn tab(&self) -> &TabInfo {
        &self.snapshot.tabs[self.index]
    }
}

impl LiveFiles {
    /// Read tabs from `snapshot` (the app replaces the inner `Arc`), send [`Command`]s to
    /// `commands` and call `wake` after each one so the app looks at the channel.
    pub fn new(
        snapshot: Arc<RwLock<Arc<Snapshot>>>,
        commands: Sender<Command>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            snapshot,
            commands,
            wake,
            opens_links: false,
            rendering: Mutex::default(),
        }
    }

    /// The host answers [`Command::OpenLink`]: `reveal_node` lets it resolve every figma.com
    /// link (open tabs and local copies alike) instead of matching tabs itself.
    #[must_use]
    pub fn with_link_opening(mut self) -> Self {
        self.opens_links = true;
        self
    }

    /// Whether the host answers [`Command::OpenLink`].
    pub fn opens_links(&self) -> bool {
        self.opens_links
    }

    fn current(&self) -> Arc<Snapshot> {
        self.snapshot
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Each tab's file key: its stem, or its full path when stems collide.
    pub fn keys(&self) -> Vec<String> {
        keys(&self.current().tabs)
    }

    /// The tab `key` names, or the active tab.
    pub(crate) fn pick(&self, key: Option<&str>) -> Result<Picked, McpError> {
        let snapshot = self.current();
        let tab = match key {
            Some(key) => find(&snapshot.tabs, key, |tab| &tab.path)?,
            None if snapshot.tabs.is_empty() => {
                return Err(McpError::app("no file is open in the app"));
            }
            None => snapshot
                .tabs
                .iter()
                .find(|tab| Some(tab.id) == snapshot.active)
                .ok_or_else(McpError::file_key)?,
        };
        let index = snapshot
            .tabs
            .iter()
            .position(|other| other.id == tab.id)
            .unwrap_or_default();
        let key = keys(&snapshot.tabs)
            .into_iter()
            .nth(index)
            .unwrap_or_default();
        Ok(Picked {
            key,
            snapshot,
            index,
        })
    }

    pub(crate) fn key(&self, key: Option<&str>) -> Result<String, McpError> {
        Ok(self.pick(key)?.key)
    }

    pub(crate) fn path(&self, key: Option<&str>) -> Result<PathBuf, McpError> {
        Ok(self.pick(key)?.tab().path.clone())
    }

    pub(crate) fn open(&self, key: Option<&str>) -> Result<Arc<Document>, McpError> {
        Ok(self.pick(key)?.tab().doc.clone())
    }

    /// The current snapshot, for the live tools.
    pub(crate) fn snapshot(&self) -> Arc<Snapshot> {
        self.current()
    }

    pub(crate) fn list(&self) -> Vec<FileInfo> {
        let snapshot = self.current();
        keys(&snapshot.tabs)
            .into_iter()
            .zip(&snapshot.tabs)
            .map(|(key, tab)| FileInfo {
                key,
                path: tab.path.clone(),
                pages: Some(tab.doc.pages.len()),
            })
            .collect()
    }

    /// How to render `doc`. Caches of documents no longer in the snapshot are dropped.
    pub(crate) fn rendering(&self, doc: &Document, system_fonts: bool) -> Rendering {
        let snapshot = self.current();
        let mut cache = self
            .rendering
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        cache.retain(|instance, _| {
            snapshot
                .tabs
                .iter()
                .any(|tab| tab.doc.instance == *instance)
        });
        let tab = snapshot
            .tabs
            .iter()
            .find(|tab| tab.doc.instance == doc.instance);
        let mut rendering = match tab {
            Some(tab) => cache
                .entry(doc.instance)
                .or_insert_with(|| Rendering::new(tab.path.clone(), Images::new(IMAGE_CACHE_BYTES)))
                .clone(),
            // The tab closed during the call: no lazy image reads, nothing cached.
            None => Rendering::default(),
        };
        rendering.system_fonts = system_fonts;
        rendering
    }

    /// Ask the app to show `node` in tab `tab` and wait up to [`REVEAL_TIMEOUT`].
    pub(crate) async fn reveal(&self, tab: u64, node: Guid) -> Result<(), McpError> {
        self.ask(REVEAL_TIMEOUT, |reply| Command::Reveal { tab, node, reply })
            .await
    }

    /// Ask the app to open the local copy `link` names and reveal its node; wait up to
    /// [`OPEN_TIMEOUT`]. Returns the app's one-line answer.
    pub(crate) async fn open_link(&self, link: &str) -> Result<String, McpError> {
        let link = link.to_owned();
        self.ask(OPEN_TIMEOUT, |reply| Command::OpenLink { link, reply })
            .await
    }

    /// Send the command `make` builds, wake the app and wait up to `timeout` for its answer.
    async fn ask<T>(
        &self,
        timeout: Duration,
        make: impl FnOnce(oneshot::Sender<anyhow::Result<T>>) -> Command,
    ) -> Result<T, McpError> {
        let (reply, answer) = oneshot::channel();
        let no_reply = || McpError::app("the app did not respond");
        self.commands.send(make(reply)).map_err(|_| no_reply())?;
        (self.wake)();
        match tokio::time::timeout(timeout, answer).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(e))) => Err(McpError::app(figstudio_fig::one_line(e.as_ref()))),
            Ok(Err(_)) | Err(_) => Err(no_reply()),
        }
    }
}

fn keys(tabs: &[TabInfo]) -> Vec<String> {
    tabs.iter()
        .map(|tab| {
            let key = stem(&tab.path);
            if tabs.iter().filter(|other| stem(&other.path) == key).count() > 1 {
                tab.path.to_string_lossy().into_owned()
            } else {
                key
            }
        })
        .collect()
}
