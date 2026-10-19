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
//! a message that says so, instead of the MCP call timing out with no reason.
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
    /// Finished searches: the search id and what it found.
    found_tx: Sender<(u64, Vec<(PathBuf, Match)>)>,
    found_rx: Receiver<(u64, Vec<(PathBuf, Match)>)>,
    /// What lists a folder, and how long it may take (tests swap both).
    pub search: SearchFn,
    pub search_timeout: Duration,
    waiting: Vec<Waiting>,
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
            found_tx,
            found_rx,
            search: local_copies,
            search_timeout: SEARCH_TIMEOUT,
            waiting: Vec::new(),
            answers: Vec::new(),
        }
    }
}

impl LinkOpen {
    /// How many folder searches are still running.
    pub(crate) fn searching(&self) -> usize {
        self.searches.len()
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
    started: web_time::Instant,
}

/// A link whose local copy is loading.
struct Waiting {
    path: PathBuf,
    node: Option<Guid>,
    done: Option<Done>,
}

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
            Self::SearchTimeout => "the app's link folders could not be read in time; macOS \
                 may be asking in the figstudio window for permission to read them"
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

/// Where a link that names `file` is shown, given the open tabs (`(id, path)`), the active
/// tab and the local `copies` found in `dirs` ([`local_copies`]). With no file named, the
/// active tab. Otherwise the one best-matching file among the tabs and the copies; a copy
/// that is an open tab counts once, as the tab. No I/O but resolving paths of namesakes.
pub(crate) fn plan(
    tabs: &[(u64, &Path)],
    active: Option<u64>,
    file: Option<&FileHint>,
    copies: Vec<(PathBuf, Match)>,
    dirs: &[PathBuf],
) -> Result<Target, LinkError> {
    let Some(hint) = file else {
        return active.map(Target::Tab).ok_or(LinkError::NoFile);
    };
    let rank = |path: &Path| hint.match_stem(path.file_stem()?.to_str()?);
    let mut candidates: Vec<(Choice, Match)> = tabs
        .iter()
        .filter_map(|&(id, path)| {
            let choice = Choice {
                target: Target::Tab(id),
                path: path.to_owned(),
            };
            Some((choice, rank(path)?))
        })
        .collect();
    for (path, rank) in copies {
        if !tabs.iter().any(|(_, open)| same_file(open, &path)) {
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

fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(a), Ok(b)) if a == b
        )
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
        self.link.choices.clear();
        let parsed = match parse(text) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.answer(ctx, done, Err(error));
                return false;
            }
        };
        let Some(hint) = parsed.file else {
            let active = self.tabs.get(self.active).map(|t| t.id);
            return match plan(&[], active, None, Vec::new(), &[]) {
                Ok(target) => self.go_to(ctx, target, parsed.node, done),
                Err(error) => {
                    self.answer(ctx, done, Err(error));
                    false
                }
            };
        };
        let id = self.link.next_search;
        self.link.next_search += 1;
        self.start_search(ctx, id, hint.clone());
        self.link.searches.push(Search {
            id,
            text: text.trim().to_owned(),
            hint,
            node: parsed.node,
            done,
            started: web_time::Instant::now(),
        });
        true
    }

    /// List the link folders for `hint` off the UI thread (in the browser there are none).
    fn start_search(&self, ctx: &egui::Context, id: u64, hint: FileHint) {
        let (dirs, search, found) = (
            self.link.dirs.clone(),
            self.link.search,
            self.link.found_tx.clone(),
        );
        let run = move || {
            // The app may have quit meanwhile; nobody to tell then.
            let _ = found.send((id, search(&dirs, &hint)));
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("link-search".into())
                .spawn(move || {
                    run();
                    ctx.request_repaint();
                });
            if let Err(error) = spawned {
                // No thread: the search times out with its message.
                tracing::warn!("link folder search not started: {error}");
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            run();
        }
    }

    /// Each frame: finish the searches that came back, fail the ones past their time.
    pub(crate) fn poll_link_searches(&mut self, ctx: &egui::Context) {
        while let Ok((id, copies)) = self.link.found_rx.try_recv() {
            let Some(i) = self.link.searches.iter().position(|s| s.id == id) else {
                continue; // Timed out before; already answered.
            };
            let search = self.link.searches.remove(i);
            self.finish_search(ctx, search, copies);
        }
        let timeout = self.link.search_timeout;
        let (late, running): (Vec<Search>, Vec<Search>) = std::mem::take(&mut self.link.searches)
            .into_iter()
            .partition(|s| s.started.elapsed() >= timeout);
        self.link.searches = running;
        for search in late {
            self.answer(ctx, search.done, Err(LinkError::SearchTimeout));
        }
        if !self.link.searches.is_empty() {
            // Wake for the timeout even if the thread never reports.
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn finish_search(
        &mut self,
        ctx: &egui::Context,
        search: Search,
        copies: Vec<(PathBuf, Match)>,
    ) {
        let tabs: Vec<(u64, &Path)> = self.tabs.iter().map(|t| (t.id, t.path.as_path())).collect();
        let active = self.tabs.get(self.active).map(|t| t.id);
        match plan(&tabs, active, Some(&search.hint), copies, &self.link.dirs) {
            Ok(target) => {
                self.go_to(ctx, target, search.node, search.done);
            }
            Err(LinkError::Several(choices)) if search.done.is_none() => {
                self.link.choices = choices;
                self.link.choice_node = search.node;
                self.link.text = search.text;
                self.link.window = true;
            }
            Err(error) => self.answer(ctx, search.done, Err(error)),
        }
    }

    /// Show `node` in `target`: now for a tab, once loaded for a local copy.
    fn go_to(
        &mut self,
        ctx: &egui::Context,
        target: Target,
        node: Option<Guid>,
        done: Option<Done>,
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
            self.answer(ctx, waiting.done, result);
        }
    }

    /// A file failed to load: the links waiting for it fail with it.
    pub(crate) fn link_failed(&mut self, path: &Path) {
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
                if self.link.searching() > 0 {
                    ui.label(
                        egui::RichText::new("Searching the link folders\u{2026}")
                            .color(crate::theme::TEXT_DIM),
                    );
                }
                picked = self.link_choices(ui);
                #[cfg(not(target_arch = "wasm32"))]
                self.link_folders(ui);
            });
        self.link.window = open;
        if let Some(target) = picked {
            let node = self.link.choice_node;
            self.link.choices.clear();
            if self.go_to(ctx, target, node, None) {
                self.link.window = false;
                self.link.text.clear();
            }
        } else if go {
            let text = std::mem::take(&mut self.link.text);
            if self.go_to_link(ctx, &text, None) {
                self.link.window = false;
            } else {
                self.link.text = text;
            }
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
                    .color(crate::theme::TEXT_DIM),
            );
        }
        picked
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn link_folders(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label("A file no tab shows is opened from its <file name>.fig in:");
        if self.link.dirs.is_empty() {
            ui.label(egui::RichText::new("(no folder)").color(crate::theme::TEXT_DIM));
        }
        for dir in &self.link.dirs {
            ui.label(egui::RichText::new(dir.display().to_string()).color(crate::theme::TEXT_DIM));
        }
        if ui.button("Choose folder\u{2026}").clicked()
            && let Some(dir) = rfd::FileDialog::new().pick_folder()
        {
            self.link.dirs = vec![dir];
        }
    }
}

#[cfg(test)]
#[path = "link_open_tests.rs"]
mod tests;
