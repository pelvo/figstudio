//! File > Open Figma link…: show the node a figma.com or `figstudio://` link names.
//!
//! The tab whose file name matches the link is used; with none, the newest local copy of
//! that name in the link folders (`local_copy.rs` in mcp-core: `FIGSTUDIO_LINK_DIRS`, else
//! `~/Downloads`, or the folder chosen in the window) is opened and the node revealed once it
//! has loaded. The MCP `reveal_node` tool reaches the same code for a file no tab shows.
use std::path::{Path, PathBuf};

use figstudio_fig::Guid;
use figstudio_mcp_core::{
    link::{FileHint, parse_link},
    local_copy::{find_local_copy, link_dirs},
    node_id::{NodeId, parse_node_id},
};

use crate::App;
use crate::tab::Tab;

/// Told once what a link did (one line) or why it failed; how an MCP request gets its answer.
pub(crate) type Done = Box<dyn FnOnce(Result<String, String>) + Send>;

/// The window's state and the links waiting for their file to load.
pub(crate) struct LinkOpen {
    pub window: bool,
    pub text: String,
    /// Folders searched for local copies, in order.
    pub dirs: Vec<PathBuf>,
    /// Give the text field focus on the next frame (the window was just opened).
    pub focus: bool,
    waiting: Vec<Waiting>,
    /// Answers kept until the frame has published the new tabs to the MCP server.
    answers: Vec<(Done, Result<String, String>)>,
}

impl Default for LinkOpen {
    fn default() -> Self {
        Self {
            window: false,
            text: String::new(),
            dirs: link_dirs(),
            focus: false,
            waiting: Vec::new(),
            answers: Vec::new(),
        }
    }
}

/// A link whose local copy is loading.
struct Waiting {
    path: PathBuf,
    node: Option<Guid>,
    done: Option<Done>,
}

/// Where a link is shown.
#[derive(Debug, PartialEq)]
pub(crate) enum Plan {
    /// This open tab.
    Tab(usize),
    /// Open this local copy first.
    Open(PathBuf),
}

/// What a pasted text names: a file (if any) and a node (if any).
#[derive(Debug, PartialEq)]
pub(crate) struct Parsed {
    pub file: Option<FileHint>,
    pub node: Option<Guid>,
}

/// A figma.com or `figstudio://` link, or a bare node ID. An instance path names its outer
/// instance, as in `reveal_node`.
pub(crate) fn parse(text: &str) -> Result<Parsed, String> {
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
        .map_err(|_| "Not a figma.com or figstudio:// link, or a node ID".to_owned())
}

/// Pure: the tab that shows `file` (the active one when it matches, else the first), else the
/// newest local copy in `dirs`. No file named: the active tab.
pub(crate) fn plan(
    paths: &[&Path],
    active: usize,
    file: Option<&FileHint>,
    dirs: &[PathBuf],
) -> Result<Plan, String> {
    let Some(hint) = file else {
        return if active < paths.len() {
            Ok(Plan::Tab(active))
        } else {
            Err("The link names no file; open the file first".to_owned())
        };
    };
    let shows = |path: &Path| {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| hint.matches(stem))
    };
    if paths.get(active).is_some_and(|path| shows(path)) {
        return Ok(Plan::Tab(active));
    }
    if let Some(i) = paths.iter().position(|path| shows(path)) {
        return Ok(Plan::Tab(i));
    }
    if let Some(path) = find_local_copy(dirs, hint) {
        return Ok(Plan::Open(path));
    }
    let name = hint.name();
    Err(if dirs.is_empty() {
        format!("No open tab shows \u{201c}{name}\u{201d}; open its .fig file first")
    } else {
        let dirs: Vec<_> = dirs.iter().map(|d| d.display().to_string()).collect();
        format!(
            "No open tab or local copy \u{201c}{name}.fig\u{201d} (searched {})",
            dirs.join(", ")
        )
    })
}

/// Make tab `i` active and show `node` there: select it and fit it, or switch to it when it
/// is a page.
fn show(
    tabs: &mut [Tab],
    active: &mut usize,
    i: usize,
    node: Option<Guid>,
) -> Result<String, String> {
    let tab = &mut tabs[i];
    let title = tab.title();
    let Some(node) = node else {
        *active = i;
        return Ok(format!("Showing {title}"));
    };
    if tab.doc.pages.contains(&node) {
        tab.set_page(node);
    } else if tab.doc.node(node).is_some() {
        tab.select(Some(node));
        tab.fit_node = Some(node);
    } else {
        return Err(format!("Node {node} is not in {title}"));
    }
    *active = i;
    Ok(format!("Showing node {node} in {title}"))
}

impl App {
    /// Show what `text` links to. `done` gets the answer (MCP); without it, failures are
    /// toasts. Returns whether the link was understood and is shown or loading.
    pub(crate) fn go_to_link(
        &mut self,
        ctx: &egui::Context,
        text: &str,
        done: Option<Done>,
    ) -> bool {
        let planned = parse(text).and_then(|parsed| {
            let paths: Vec<&Path> = self.tabs.iter().map(|t| t.path.as_path()).collect();
            plan(&paths, self.active, parsed.file.as_ref(), &self.link.dirs)
                .map(|plan| (plan, parsed.node))
        });
        match planned {
            Ok((Plan::Tab(i), node)) => {
                let result = show(&mut self.tabs, &mut self.active, i, node);
                let ok = result.is_ok();
                self.answer(ctx, done, result);
                ok
            }
            Ok((Plan::Open(path), node)) => {
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
            Err(error) => {
                self.answer(ctx, done, Err(error));
                false
            }
        }
    }

    /// A file finished loading as tab `i`: show the links that were waiting for it.
    pub(crate) fn link_loaded(&mut self, ctx: &egui::Context, path: &Path, i: usize) {
        for waiting in self.take_waiting(path) {
            let result = show(&mut self.tabs, &mut self.active, i, waiting.node);
            self.answer(ctx, waiting.done, result);
        }
    }

    /// A file failed to load: the links waiting for it fail with it.
    pub(crate) fn link_failed(&mut self, path: &Path, message: &str) {
        for waiting in self.take_waiting(path) {
            // Without `done` the load error is already a toast.
            if let Some(done) = waiting.done {
                self.link.answers.push((done, Err(message.to_owned())));
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

    fn answer(&mut self, ctx: &egui::Context, done: Option<Done>, result: Result<String, String>) {
        match (done, result) {
            (Some(done), result) => self.link.answers.push((done, result)),
            (None, Err(error)) => self.toast(ctx, error, true),
            (None, Ok(_)) => {}
        }
    }

    /// End of frame, after the MCP snapshot is published: send the answers.
    pub(crate) fn send_link_answers(&mut self) {
        for (done, result) in self.link.answers.drain(..) {
            done(result);
        }
    }

    pub(crate) fn link_window(&mut self, ctx: &egui::Context) {
        let mut open = self.link.window;
        let mut go = false;
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
                #[cfg(not(target_arch = "wasm32"))]
                self.link_folders(ui);
            });
        self.link.window = open;
        if go {
            let text = std::mem::take(&mut self.link.text);
            if self.go_to_link(ctx, &text, None) {
                self.link.window = false;
            } else {
                self.link.text = text;
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn link_folders(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label("A file no tab shows is opened from the newest <file name>.fig in:");
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
