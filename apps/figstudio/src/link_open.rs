//! File > Open Figma link…: show the node a figma.com or `figstudio://` link names.
//!
//! A `.fig` file does not store its figma.com key, so the file is found by name: every open
//! tab and every `.fig` directly inside the link folders (`local_copy.rs` in mcp-core:
//! `FIGSTUDIO_LINK_DIRS`, else `~/Downloads`, or the folder chosen in the window) that
//! matches is a candidate. A key match beats a name match; when several distinct files tie,
//! the window lists them for a person to pick, and nothing is guessed. A local copy is opened
//! and the node revealed once it has loaded. The MCP `reveal_node` tool reaches the same code
//! for every figma.com link; its answers never carry paths or file names ([`LinkError::public`]).
//!
//! The folders are listed on a background thread ([`LinkOpen::search`]) and the result is
//! picked up by [`App::poll_link_searches`] each frame: reading a folder can block for long
//! (a network drive, or macOS asking for permission to read `~/Downloads`), and the window
//! must keep drawing meanwhile. A search that takes longer than [`SEARCH_TIMEOUT`] fails with
//! a message that says so, instead of the MCP call timing out with no reason. A new link from
//! the window replaces the window's older search (its result is dropped); MCP calls each keep
//! their own. While a timed-out listing is still blocked, new links fail at once rather than
//! piling up more blocked threads.
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use figstudio_fig::Guid;
use figstudio_mcp_core::{
    link::{FileHint, Match, best_matches, parse_link},
    local_copy::{link_dirs, local_copies},
    node_id::{NodeId, parse_node_id},
};

use crate::App;
use crate::tab::Tab;

/// Told once what a link did (one line) or why it failed; how an MCP request gets its answer.
/// Both texts are [`LinkError::public`]-safe: no paths, folder names or file names.
pub(crate) type Done = Box<dyn FnOnce(Result<String, String>) + Send>;

/// How long a folder search may take before the link fails (macOS may be waiting for the
/// person to allow reading the folder; the search then still finishes in the background).
pub(crate) const SEARCH_TIMEOUT: Duration = Duration::from_secs(20);

/// How long an MCP request may wait for its local copy to load, counted from the request:
/// it is answered before the server gives up (`figstudio_mcp::live::OPEN_TIMEOUT`, 60 s), and
/// the file still opens and shows the node afterwards.
pub(crate) const ANSWER_BY: Duration = Duration::from_secs(55);

/// A link an MCP caller asked the app to show, and when it asked (its deadline counts from
/// then, not from when the app took the request).
pub(crate) struct LinkAsk {
    pub link: String,
    pub asked: web_time::Instant,
    pub done: Done,
}

/// How many listings may be given up on (their folders chosen again or away from) and be
/// stuck, past [`SEARCH_TIMEOUT`] and still not back, before new links are refused rather
/// than start yet another thread. Listings that are merely slow do not count.
pub(crate) const MAX_ABANDONED: usize = 3;

/// A finished listing: its search id, what it found, and when it finished.
type Found = (u64, Vec<Copy>, web_time::Instant);

/// Lists the local copies of a link in some folders; [`local_copies`] unless a test swaps it.
pub(crate) type SearchFn = fn(&[PathBuf], &FileHint) -> Vec<(PathBuf, Match)>;

/// A [`Done`] with the answer it is to get once the frame has published its tabs.
pub(crate) type Answer = (Done, Result<String, String>);

/// The window's state and the links waiting for their file to load.
pub(crate) struct LinkOpen {
    pub window: bool,
    pub text: String,
    /// Folders searched for local copies, in order.
    pub dirs: Vec<PathBuf>,
    /// Give the text field focus on the next frame (the window was just opened).
    pub focus: bool,
    /// Files that tie for the last link, for a person to pick in the window.
    pub choices: Vec<Choice>,
    /// The node of the link the choices are for.
    choice_node: Option<Guid>,
    /// Folder searches running on their threads.
    searches: Vec<Search>,
    next_search: u64,
    /// Listings still running (search id, start), timed-out ones included: one that is past
    /// [`SEARCH_TIMEOUT`] and still blocked makes new links fail at once.
    listings: Vec<(u64, web_time::Instant)>,
    /// Listings given up on by choosing folders, not back yet (id, start): they no longer block
    /// links, but at [`MAX_ABANDONED`] stuck ones new links are refused (each would leave
    /// another thread waiting).
    abandoned: Vec<(u64, web_time::Instant)>,
    /// Finished searches: the search id and what it found.
    found_tx: Sender<Found>,
    found_rx: Receiver<Found>,
    /// What lists a folder, and how long it may take (tests swap both).
    pub search: SearchFn,
    pub search_timeout: Duration,
    /// [`ANSWER_BY`] unless a test swaps it.
    pub answer_by: Duration,
    waiting: Vec<Waiting>,
    /// Resolved paths of files that failed to load while searches ran: a copy that is one of
    /// them is not opened again. Cleared once no search runs.
    failed: Vec<PathBuf>,
    /// Answers kept until the frame has published the new tabs to the MCP server.
    answers: Vec<Answer>,
}

impl Default for LinkOpen {
    fn default() -> Self {
        let (found_tx, found_rx) = channel();
        Self {
            window: false,
            text: String::new(),
            dirs: link_dirs(),
            focus: false,
            choices: Vec::new(),
            choice_node: None,
            searches: Vec::new(),
            next_search: 0,
            listings: Vec::new(),
            abandoned: Vec::new(),
            found_tx,
            found_rx,
            search: local_copies,
            search_timeout: SEARCH_TIMEOUT,
            answer_by: ANSWER_BY,
            waiting: Vec::new(),
            failed: Vec::new(),
            answers: Vec::new(),
        }
    }
}

impl LinkOpen {
    /// How many folder searches are still running.
    #[cfg(test)]
    pub(crate) fn searching(&self) -> usize {
        self.searches.len()
    }

    /// Whether the window's own link is being searched for.
    fn window_searching(&self) -> bool {
        self.searches.iter().any(Search::is_window)
    }

    /// Why a new link must not start a listing: one of the current folders' listings is
    /// past the timeout and still blocked, or [`MAX_ABANDONED`] given-up ones are.
    fn blocked(&self) -> Option<LinkError> {
        let timeout = self.search_timeout;
        let stuck = |(_, started): &&(u64, web_time::Instant)| started.elapsed() >= timeout;
        if self.abandoned.iter().filter(stuck).count() >= MAX_ABANDONED {
            Some(LinkError::ListingLimit)
        } else if self.listings.iter().any(|l| stuck(&l)) {
            Some(LinkError::SearchTimeout)
        } else {
            None
        }
    }

    /// Search `dirs` from now on. Every running listing is given up on, the same folder's
    /// too: choosing a folder is how a person retries one that hung (a share that has woken),
    /// so a stuck listing no longer blocks links. Up to [`MAX_ABANDONED`] may be waiting.
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the browser has no link folders to choose")
    )]
    pub(crate) fn set_dirs(&mut self, dirs: Vec<PathBuf>) {
        self.abandoned.append(&mut self.listings);
        self.dirs = dirs;
    }
}

/// A link whose folders are being searched.
struct Search {
    id: u64,
    /// The link as given, for the window when the result is a choice.
    text: String,
    hint: FileHint,
    node: Option<Guid>,
    done: Option<Done>,
    /// When the listing started, for [`SEARCH_TIMEOUT`].
    started: web_time::Instant,
    /// When the link was asked for (an MCP caller's send time), for [`ANSWER_BY`].
    asked: web_time::Instant,
    /// What the listing found, once it is back. Held while a file is loading: that file is
    /// not a tab yet, and its tab's real path is needed to tell it from a copy.
    found: Option<Vec<Copy>>,
    /// An MCP search already answered (its budget ran out while it was held for a load); its
    /// `done` is gone but it is not the window's. It still shows the node when it finishes.
    answered: bool,
}

impl Search {
    /// Whether this is the Open Figma link window's own search.
    fn is_window(&self) -> bool {
        self.done.is_none() && !self.answered
    }
}

/// A link whose local copy is loading.
struct Waiting {
    path: PathBuf,
    node: Option<Guid>,
    done: Option<Done>,
    /// When the link was asked for, for [`ANSWER_BY`].
    asked: web_time::Instant,
    /// Its MCP caller was answered already: show the node, say nothing more.
    answered: bool,
}

/// A local copy of a link: the path it was listed under (opened and shown), the same path
/// resolved (to recognise it as an open tab), and how well it matches.
pub(crate) type Copy = (PathBuf, PathBuf, Match);

/// Where a link is shown.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Target {
    /// The open tab with this id.
    Tab(u64),
    /// Open this local copy first.
    Open(PathBuf),
}

/// One of several files a link could mean: where it is shown and its path, for the label.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Choice {
    pub target: Target,
    pub path: PathBuf,
}

/// Why a link cannot be shown.
#[derive(Debug, PartialEq)]
pub(crate) enum LinkError {
    /// Not a link or a node ID.
    Unreadable,
    /// The link names no file and no tab is open.
    NoFile,
    /// No open tab or local copy matches.
    NotFound { name: String, dirs: Vec<PathBuf> },
    /// Several distinct files match equally well.
    Several(Vec<Choice>),
    /// The file does not have the node.
    NodeMissing { node: Guid, title: String },
    /// The tab a choice named was closed meanwhile.
    TabClosed,
    /// The local copy did not load.
    LoadFailed,
    /// Listing the link folders took longer than [`SEARCH_TIMEOUT`].
    SearchTimeout,
    /// Too many abandoned folder listings are still stuck; no more threads may start.
    ListingLimit,
    /// The folders answered, but a file the link may name was still loading at the timeout.
    StillLoading,
}

impl LinkError {
    /// For the person at this computer: names, folders and paths included.
    pub(crate) fn local(&self) -> String {
        match self {
            Self::NotFound { name, dirs } if dirs.is_empty() => {
                format!("No open tab shows \u{201c}{name}\u{201d}; open its .fig file first")
            }
            Self::NotFound { name, dirs } => {
                let dirs: Vec<_> = dirs.iter().map(|d| d.display().to_string()).collect();
                format!(
                    "No open tab or local copy \u{201c}{name}.fig\u{201d} (searched {})",
                    dirs.join(", ")
                )
            }
            Self::Several(choices) => {
                format!("{} files match this link; pick one", choices.len())
            }
            Self::NodeMissing { node, title } => format!("Node {node} is not in {title}"),
            other => other.public(),
        }
    }

    /// For an MCP client: what happened, without paths, folder names or file names.
    pub(crate) fn public(&self) -> String {
        match self {
            Self::Unreadable => "not a figma.com or figstudio:// link, or a node ID".to_owned(),
            Self::NoFile => "the link names no file and no tab is open".to_owned(),
            Self::NotFound { .. } => {
                "no open tab or local copy of this file was found in the app's link folders"
                    .to_owned()
            }
            Self::Several(_) => "several files match this link; pick one in figstudio with \
                 File \u{2192} Open Figma link\u{2026}, then call reveal_node again"
                .to_owned(),
            Self::NodeMissing { node, .. } => format!("node {node} is not in that file"),
            Self::TabClosed => "tab closed".to_owned(),
            Self::LoadFailed => "the local copy could not be opened".to_owned(),
            Self::SearchTimeout if cfg!(target_os = "macos") => {
                "the app's link folders could not be read in time; macOS may be asking in the \
                 figstudio window for permission to read them"
                    .to_owned()
            }
            Self::ListingLimit => {
                let mut text = "too many folder searches are still stuck; wait for them to \
                    finish or restart figstudio to try again"
                    .to_owned();
                if cfg!(target_os = "macos") {
                    text.push_str(
                        "; macOS may be asking in the figstudio window for permission \
                        to read the link folders",
                    );
                }
                text
            }
            Self::StillLoading => "a file this link may name is still opening in figstudio; \
                 try again once it has opened"
                .to_owned(),
            Self::SearchTimeout => "the app's link folders could not be read in time (a slow \
                 or unreachable folder)"
                .to_owned(),
        }
    }
}

/// What a pasted text names: a file (if any) and a node (if any).
#[derive(Debug, PartialEq)]
pub(crate) struct Parsed {
    pub file: Option<FileHint>,
    pub node: Option<Guid>,
}

/// A figma.com or `figstudio://` link, or a bare node ID. An instance path names its outer
/// instance, as in `reveal_node`.
pub(crate) fn parse(text: &str) -> Result<Parsed, LinkError> {
    let text = text.trim();
    let node = |id: NodeId| match id {
        NodeId::Node(guid) => guid,
        NodeId::InstancePath(path) => path.instance(),
    };
    if let Ok(link) = parse_link(text) {
        let node = link.node.map(node).or(link.page);
        return Ok(Parsed {
            file: link.file,
            node,
        });
    }
    parse_node_id(text)
        .map(|id| Parsed {
            file: None,
            node: Some(node(id)),
        })
        .map_err(|_| LinkError::Unreadable)
}

/// Where a link that names `file` is shown, given the open tabs (`(id, path, real path)`),
/// the active tab and the local `copies` found in `dirs` ([`local_copies`], resolved by
/// [`real_paths`]). With no file named, the active tab. Otherwise the one best-matching file
/// among the tabs and the copies; a copy that is an open tab (equal to its path or its real
/// path) counts once, as the tab. No I/O, so it is safe on the UI thread.
pub(crate) fn plan(
    tabs: &[(u64, &Path, &Path)],
    active: Option<u64>,
    file: Option<&FileHint>,
    copies: Vec<Copy>,
    dirs: &[PathBuf],
) -> Result<Target, LinkError> {
    let Some(hint) = file else {
        return active.map(Target::Tab).ok_or(LinkError::NoFile);
    };
    let rank = |path: &Path| hint.match_stem(path.file_stem()?.to_str()?);
    let mut candidates: Vec<(Choice, Match)> = tabs
        .iter()
        .filter_map(|&(id, path, _)| {
            let choice = Choice {
                target: Target::Tab(id),
                path: path.to_owned(),
            };
            Some((choice, rank(path)?))
        })
        .collect();
    for (path, real, rank) in copies {
        let open = |&(_, tab, tab_real): &(u64, &Path, &Path)| {
            tab == path || tab_real == real || tab == real
        };
        if !tabs.iter().any(open) {
            let target = Target::Open(path.clone());
            candidates.push((Choice { target, path }, rank));
        }
    }
    let mut best = best_matches(candidates);
    match best.len() {
        0 => Err(LinkError::NotFound {
            name: hint.name().to_owned(),
            dirs: dirs.to_vec(),
        }),
        1 => Ok(best.remove(0).target),
        _ => Err(LinkError::Several(best)),
    }
}

/// The `copies` of a link, each with its path resolved (symlinks, `..`) to compare with a
/// tab's [`Tab::real_path`]. Disk access: runs with the folder listing, off the UI thread.
pub(crate) fn real_paths(copies: Vec<(PathBuf, Match)>) -> Vec<Copy> {
    copies
        .into_iter()
        .map(|(path, rank)| {
            let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            (path, real, rank)
        })
        .collect()
}

/// `path` with `.` and `..` folded away by its text alone (no disk access, so symlinks are
/// not followed): how a failed load is matched when the worker could not resolve it. A `..`
/// removes a normal segment before it, is dropped when there is none above a root (`/..`
/// is `/`, `C:\..` is `C:\`), and otherwise stays (`../../a` keeps both; so does Windows'
/// rootless `C:..`, relative to that drive's current folder).
pub(crate) fn lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                _ if out.has_root() => {}
                _ => out.push(".."),
            },
            other => out.push(other),
        }
    }
    out
}

/// Whether `path`'s file name is one `hint` matches (no disk access).
fn names(hint: &FileHint, path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| hint.match_stem(stem).is_some())
}

/// Make tab `i` active and show `node` there ([`Tab::show_node`]). The answer reads like
/// `reveal_node`'s: `revealed 1:2 in tab 3`.
fn show(
    tabs: &mut [Tab],
    active: &mut usize,
    i: usize,
    node: Option<Guid>,
) -> Result<String, LinkError> {
    let tab = &mut tabs[i];
    let Some(node) = node else {
        *active = i;
        return Ok(format!("showing tab {}", tab.id));
    };
    if tab.doc.node(node).is_none() {
        return Err(LinkError::NodeMissing {
            node,
            title: tab.title(),
        });
    }
    tab.show_node(node);
    *active = i;
    Ok(format!("revealed {node} in tab {}", tab.id))
}

impl App {
    /// Show what `text` links to. `done` gets the answer (MCP); without it, failures are
    /// toasts and a tie fills the window's choices. A link that names a file starts a folder
    /// search and is finished by [`App::poll_link_searches`]. Returns whether the link is
    /// shown, loading or being searched for.
    pub(crate) fn go_to_link(
        &mut self,
        ctx: &egui::Context,
        text: &str,
        done: Option<Done>,
    ) -> bool {
        self.go_to_link_at(ctx, text, done, web_time::Instant::now())
    }

    /// [`Self::go_to_link`] for a link asked for at `asked`: an MCP request, whose
    /// [`ANSWER_BY`] counts from when the caller sent it.
    pub(crate) fn go_to_link_at(
        &mut self,
        ctx: &egui::Context,
        text: &str,
        done: Option<Done>,
        asked: web_time::Instant,
    ) -> bool {
        self.link.choices.clear();
        let parsed = match parse(text) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.answer(ctx, done, Err(error));
                return false;
            }
        };
        let window = done.is_none();
        let Some(hint) = parsed.file else {
            let active = self.tabs.get(self.active).map(|t| t.id);
            return match plan(&[], active, None, Vec::new(), &[]) {
                Ok(target) => {
                    if window {
                        // The window's link is now this one, shown or refused (a node the
                        // file does not have); its older search is moot.
                        self.cancel_window_search();
                    }
                    self.go_to(ctx, target, parsed.node, done, asked)
                }
                Err(error) => {
                    self.answer(ctx, done, Err(error));
                    false
                }
            };
        };
        let text = text.trim().to_owned();
        if window
            && self
                .link
                .searches
                .iter()
                .any(|s| s.is_window() && s.text == text)
        {
            // Go again on the link being searched for: keep that search, start no thread.
            return true;
        }
        if let Some(error) = self.link.blocked() {
            // A stuck current listing reports a folder timeout; the abandoned-listing cap
            // reports wait/restart guidance. Neither starts a thread or cancels the window's search.
            self.answer(ctx, done, Err(error));
            return false;
        }
        if window {
            // A new link from the window replaces the window's older one.
            self.cancel_window_search();
        }
        let id = self.link.next_search;
        self.link.next_search += 1;
        self.start_search(ctx, id, hint.clone());
        self.link.searches.push(Search {
            id,
            text,
            hint,
            node: parsed.node,
            done,
            started: web_time::Instant::now(),
            asked,
            found: None,
            answered: false,
        });
        true
    }

    /// Drop the window's search, if any: its result is then ignored when it comes (MCP
    /// searches are untouched).
    pub(crate) fn cancel_window_search(&mut self) {
        self.link.searches.retain(|s| !s.is_window());
    }

    /// List the link folders for `hint` off the UI thread (in the browser there are none,
    /// and it runs inline). The result is applied by the next frames' [`Self::poll_link_searches`].
    fn start_search(&mut self, ctx: &egui::Context, id: u64, hint: FileHint) {
        let search = self.link.search;
        let dirs = self.link.dirs.clone();
        let found = self.link.found_tx.clone();
        self.link.listings.push((id, web_time::Instant::now()));
        let list = move || {
            let copies = real_paths(search(&dirs, &hint));
            // The app may have quit meanwhile; nobody to tell then.
            let _ = found.send((id, copies, web_time::Instant::now()));
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (list, ctx) = (
                std::sync::Arc::new(std::sync::Mutex::new(Some(list))),
                ctx.clone(),
            );
            let job = std::sync::Arc::clone(&list);
            let spawned = std::thread::Builder::new()
                .name("link-search".into())
                .spawn(move || {
                    if let Some(list) = job.lock().ok().and_then(|mut l| l.take()) {
                        list();
                    }
                    ctx.request_repaint();
                });
            if let Err(error) = spawned {
                // No thread: list here rather than wait for the timeout.
                tracing::warn!("link folder search thread not started: {error}");
                if let Some(list) = list.lock().ok().and_then(|mut l| l.take()) {
                    list();
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        list();
        // The next frame polls, and keeps polling until the result or the timeout.
        ctx.request_repaint();
    }

    /// Each frame: take in the listings that came back (one that finished after its
    /// deadline counts as timed out, however soon it is polled), fail the searches past their
    /// time, and finish the rest once no file they could name is loading. A result for a
    /// search that is gone (timed out, replaced or cancelled) is dropped. Also answers MCP
    /// requests still waiting for a load at [`ANSWER_BY`].
    pub(crate) fn poll_link_searches(&mut self, ctx: &egui::Context) {
        let timeout = self.link.search_timeout;
        while let Ok((id, copies, finished)) = self.link.found_rx.try_recv() {
            self.link.listings.retain(|(listing, _)| *listing != id);
            self.link.abandoned.retain(|(listing, _)| *listing != id);
            if let Some(search) = self.link.searches.iter_mut().find(|s| s.id == id)
                && finished.saturating_duration_since(search.started) < timeout
            {
                search.found = Some(copies);
            }
        }
        // A file still loading whose name the link matches may be one of the copies under
        // another spelling; once it is a tab, its real path tells. The worker wakes the UI when
        // the load ends; a load that never ends runs into the timeout below.
        let loading: Vec<PathBuf> = self.loading.iter().cloned().collect();
        let (ready, waiting): (Vec<Search>, Vec<Search>) = std::mem::take(&mut self.link.searches)
            .into_iter()
            .partition(|s| s.found.is_some() && !loading.iter().any(|p| names(&s.hint, p)));
        self.link.searches = waiting;
        for search in ready {
            self.finish_search(ctx, search);
        }
        // Past the listing's own deadline, or past the MCP caller's budget (counted from when it
        // asked, so a request that waited before a frame took it is answered in time even while
        // its folders are still being listed).
        let answer_by = self.link.answer_by;
        for mut search in std::mem::take(&mut self.link.searches) {
            if search.answered {
                // Its caller was told; it only waits for the load to show the node, as a
                // waiting load does (no timeout of its own, no second answer).
                self.link.searches.push(search);
                continue;
            }
            let over_budget = search.done.is_some() && search.asked.elapsed() >= answer_by;
            if search.started.elapsed() >= timeout || (over_budget && search.found.is_none()) {
                // Held for a load past its time, or not back (in time) at all: give up.
                let error = match search.found {
                    Some(_) => LinkError::StillLoading,
                    None => LinkError::SearchTimeout,
                };
                self.answer(ctx, search.done, Err(error));
            } else if over_budget {
                // The listing is in hand and a matching file is still opening: answer the
                // caller now, and still show the node once the load ends (as for a load
                // already waiting).
                self.answer(ctx, search.done.take(), Err(LinkError::StillLoading));
                search.answered = true;
                self.link.searches.push(search);
            } else {
                self.link.searches.push(search);
            }
        }
        if self.link.searches.is_empty() {
            self.link.failed.clear();
        }
        // A load that outlasts the caller's budget: answer now; the file still opens and
        // shows the node (the entry stays, without its `done`).
        let overdue: Vec<Done> = self
            .link
            .waiting
            .iter_mut()
            .filter(|w| w.asked.elapsed() >= answer_by)
            .filter_map(|w| {
                let done = w.done.take()?;
                w.answered = true;
                Some(done)
            })
            .collect();
        for done in overdue {
            self.answer(ctx, Some(done), Err(LinkError::StillLoading));
        }
        if !self.link.searches.is_empty() || self.link.waiting.iter().any(|w| w.done.is_some()) {
            // Wake for the timeouts even if nothing else reports.
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn finish_search(&mut self, ctx: &egui::Context, mut search: Search) {
        let copies = search.found.take().unwrap_or_default();
        // Copies that just failed to load, by either spelling (the worker may not have
        // resolved the path). They still take part in the ranking, so a worse match or the
        // other half of a tie never takes their place; only opening one is refused.
        let failed: Vec<PathBuf> = copies
            .iter()
            .filter(|(path, real, _)| {
                self.link.failed.iter().any(|f| {
                    let f = lexical(f);
                    f == lexical(path) || f == lexical(real)
                })
            })
            .map(|(path, _, _)| path.clone())
            .collect();
        // The tabs open now, not when the search started: one may have opened meanwhile.
        let tabs: Vec<(u64, &Path, &Path)> = self
            .tabs
            .iter()
            .map(|t| (t.id, t.path.as_path(), t.real_path.as_path()))
            .collect();
        let active = self.tabs.get(self.active).map(|t| t.id);
        let planned = plan(&tabs, active, Some(&search.hint), copies, &self.link.dirs);
        if search.answered {
            // Its caller was told already: show the node if there is one file to show it in,
            // and otherwise stay quiet (no toast, no window, no second answer).
            self.show_quietly(planned, &failed, search.node, search.asked);
            return;
        }
        match planned {
            Ok(Target::Open(path)) if failed.contains(&path) => {
                // Not decoded again; its load error is already a toast.
                self.answer(ctx, search.done, Err(LinkError::LoadFailed));
            }
            Ok(target) => {
                let window = search.is_window();
                if self.go_to(ctx, target, search.node, search.done, search.asked) && window {
                    // Shown or loading: the window's job is done.
                    self.link.window = false;
                    if self.link.text.trim() == search.text {
                        self.link.text.clear();
                    }
                }
            }
            Err(LinkError::Several(choices)) if search.is_window() => {
                self.link.choices = choices;
                self.link.choice_node = search.node;
                self.link.text = search.text;
                self.link.window = true;
            }
            Err(error) => self.answer(ctx, search.done, Err(error)),
        }
    }

    /// For an answered search: show `node` in the planned file, with no toast and no answer.
    fn show_quietly(
        &mut self,
        planned: Result<Target, LinkError>,
        failed: &[PathBuf],
        node: Option<Guid>,
        asked: web_time::Instant,
    ) {
        match planned {
            Ok(Target::Tab(id)) => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
                    let _ = show(&mut self.tabs, &mut self.active, i, node);
                }
            }
            Ok(Target::Open(path)) if !failed.contains(&path) => {
                self.link.waiting.push(Waiting {
                    path: path.clone(),
                    node,
                    done: None,
                    asked,
                    answered: true,
                });
                self.open(path);
            }
            _ => {}
        }
    }

    /// Show `node` in `target`: now for a tab, once loaded for a local copy.
    fn go_to(
        &mut self,
        ctx: &egui::Context,
        target: Target,
        node: Option<Guid>,
        done: Option<Done>,
        asked: web_time::Instant,
    ) -> bool {
        match target {
            Target::Tab(id) => {
                let result = match self.tabs.iter().position(|t| t.id == id) {
                    Some(i) => show(&mut self.tabs, &mut self.active, i, node),
                    None => Err(LinkError::TabClosed),
                };
                let ok = result.is_ok();
                self.answer(ctx, done, result);
                ok
            }
            Target::Open(path) => {
                if done.is_none() {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    self.toast(ctx, format!("Opening {name}\u{2026}"), false);
                }
                self.link.waiting.push(Waiting {
                    path: path.clone(),
                    node,
                    done,
                    asked,
                    answered: false,
                });
                self.open(path);
                true
            }
        }
    }

    /// A file finished loading as tab `i`: show the links that were waiting for it.
    pub(crate) fn link_loaded(&mut self, ctx: &egui::Context, path: &Path, i: usize) {
        for waiting in self.take_waiting(path) {
            let result = show(&mut self.tabs, &mut self.active, i, waiting.node)
                .map(|done| done + " (opened the local copy)");
            if !waiting.answered {
                self.answer(ctx, waiting.done, result);
            }
        }
    }

    /// A file failed to load: the links waiting for it fail with it.
    pub(crate) fn link_failed(&mut self, path: &Path, real_path: Option<PathBuf>) {
        if !self.link.searches.is_empty() {
            self.link
                .failed
                .push(real_path.unwrap_or_else(|| path.to_owned()));
        }
        for waiting in self.take_waiting(path) {
            // Without `done` the load error is already a toast.
            if let Some(done) = waiting.done {
                let error = LinkError::LoadFailed.public();
                self.link.answers.push((done, Err(error)));
            }
        }
    }

    fn take_waiting(&mut self, path: &Path) -> Vec<Waiting> {
        let (done, kept) = std::mem::take(&mut self.link.waiting)
            .into_iter()
            .partition(|w| w.path == path);
        self.link.waiting = kept;
        done
    }

    /// MCP gets the public text; a person gets a toast with the local one.
    fn answer(
        &mut self,
        ctx: &egui::Context,
        done: Option<Done>,
        result: Result<String, LinkError>,
    ) {
        match (done, result) {
            (Some(done), result) => {
                let result = result.map_err(|error| error.public());
                self.link.answers.push((done, result));
            }
            (None, Err(error)) => self.toast(ctx, error.local(), true),
            (None, Ok(_)) => {}
        }
    }

    /// The answers this frame produced, for `McpHost::sync_then_answer`.
    pub(crate) fn take_link_answers(&mut self) -> Vec<Answer> {
        std::mem::take(&mut self.link.answers)
    }

    /// Submit the link field, from its window or the command bar. Keep the text while
    /// searching and after a failure, and close the window once the link is shown or loading.
    pub(crate) fn submit_link(&mut self, ctx: &egui::Context) {
        let text = self.link.text.clone();
        if self.go_to_link(ctx, &text, None) && !self.link.window_searching() {
            self.link.window = false;
            self.link.text.clear();
        }
    }

    pub(crate) fn link_window(&mut self, ctx: &egui::Context) {
        let mut open = self.link.window;
        let mut go = false;
        let mut picked = None;
        egui::Window::new("Open Figma link")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label("Paste a figma.com or figstudio:// link, or a node ID.");
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.link.text)
                        .hint_text("https://www.figma.com/design/\u{2026}?node-id=\u{2026}")
                        .desired_width(460.0),
                );
                if std::mem::take(&mut self.link.focus) {
                    edit.request_focus();
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let filled = !self.link.text.trim().is_empty();
                go = ui.add_enabled(filled, egui::Button::new("Go")).clicked() || (enter && filled);
                if self.link.window_searching() {
                    ui.label(
                        egui::RichText::new("Searching the link folders\u{2026}")
                            .color(crate::theme::palette(ui.ctx()).text_dim),
                    );
                }
                picked = self.link_choices(ui);
                #[cfg(not(target_arch = "wasm32"))]
                self.link_folders(ui);
            });
        if self.link.window && !open {
            // Closed by the person: the link they asked for is no longer wanted.
            self.cancel_window_search();
        }
        self.link.window = open;
        if let Some(target) = picked {
            let node = self.link.choice_node;
            self.link.choices.clear();
            if self.go_to(ctx, target, node, None, web_time::Instant::now()) {
                self.link.window = false;
                self.link.text.clear();
            }
        } else if go {
            self.submit_link(ctx);
        }
    }

    /// The files that tie for the last link, one button each; the one clicked.
    fn link_choices(&self, ui: &mut egui::Ui) -> Option<Target> {
        if self.link.choices.is_empty() {
            return None;
        }
        ui.separator();
        ui.label("Several files match this link. Pick the one it means:");
        let mut picked = None;
        for choice in &self.link.choices {
            let name = choice
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let folder = choice
                .path
                .parent()
                .map(|dir| dir.display().to_string())
                .unwrap_or_default();
            let open = if matches!(choice.target, Target::Tab(_)) {
                " (open)"
            } else {
                ""
            };
            if ui
                .button(format!("{name}{open}"))
                .on_hover_text(&folder)
                .clicked()
            {
                picked = Some(choice.target.clone());
            }
            ui.label(
                egui::RichText::new(folder)
                    .small()
                    .color(crate::theme::palette(ui.ctx()).text_dim),
            );
        }
        picked
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn link_folders(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label("A file no tab shows is opened from its <file name>.fig in:");
        if self.link.dirs.is_empty() {
            ui.label(
                egui::RichText::new("(no folder)").color(crate::theme::palette(ui.ctx()).text_dim),
            );
        }
        for dir in &self.link.dirs {
            ui.label(
                egui::RichText::new(dir.display().to_string())
                    .color(crate::theme::palette(ui.ctx()).text_dim),
            );
        }
        if ui.button("Choose folder\u{2026}").clicked()
            && let Some(dir) = rfd::FileDialog::new().pick_folder()
        {
            self.link.set_dirs(vec![dir]);
        }
    }
}

#[cfg(test)]
#[path = "link_open_tests.rs"]
mod tests;
