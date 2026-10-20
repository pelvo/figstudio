//! Desktop workspace restoration; documents open on bounded workers, metadata stays private.
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{App, tab::Tab};

pub(crate) const MAX_SAVED_FILES: usize = 500;
const MAX_LOADING: usize = 4;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Session {
    files: Vec<PathBuf>,
    active: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct Restore {
    session: Session,
    waiting: VecDeque<PathBuf>,
    /// Distinguishes automatic focus from a different space selected while loading.
    last_active: Option<u64>,
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

impl Session {
    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Bounds stored metadata, retaining the selected file if it was beyond the cap.
    pub(crate) fn limit(&mut self) -> usize {
        let before = self.files.len();
        self.active = self.active.take().filter(|p| self.files.contains(p));
        self.files.truncate(MAX_SAVED_FILES);
        if let Some(active) = &self.active
            && !self.files.contains(active)
        {
            self.files.pop();
            self.files.push(active.clone());
        }
        before.saturating_sub(self.files.len())
    }

    fn push(&mut self, path: &Path) {
        let path = absolute(path);
        if !self.files.contains(&path) {
            self.files.push(path);
        }
    }

    pub(crate) fn capture(app: &App) -> Self {
        let mut session = Self::default();
        let current = app.tabs.get(app.active);
        let mut active = current.map(|t| t.path.clone());
        if let Some(restore) = &app.restoring {
            for path in &restore.session.files {
                if app.tabs.iter().any(|t| t.path == *path)
                    || app.loading.iter().any(|p| p == path)
                    || restore.waiting.contains(path)
                {
                    session.push(path);
                }
            }
            if current.map(|t| t.id) == restore.last_active {
                active = restore.session.active.clone().or(active);
            }
        }
        for path in app.tabs.iter().map(|t| &t.path).chain(app.loading.iter()) {
            session.push(path);
        }
        session.active = active
            .map(|p| absolute(&p))
            .filter(|p| session.files.contains(p))
            .or_else(|| {
                current
                    .map(|t| absolute(&t.path))
                    .filter(|p| session.files.contains(p))
            });
        session
    }
}

impl Restore {
    fn new(session: Session) -> Self {
        Self {
            waiting: session.files.iter().cloned().collect(),
            session,
            last_active: None,
        }
    }

    fn alias(&mut self, from: &Path, to: &Path) {
        let files = std::mem::take(&mut self.session.files);
        for path in files {
            let path = if path == from { to.to_path_buf() } else { path };
            if !self.session.files.contains(&path) {
                self.session.files.push(path);
            }
        }
        self.waiting.retain(|p| p != from && p != to);
        if self.session.active.as_deref() == Some(from) {
            self.session.active = Some(to.to_path_buf());
        }
    }

    fn closed(&mut self, closed: &Tab, previous: Option<u64>, current: Option<&Tab>) {
        self.session.files.retain(|p| *p != closed.path);
        self.waiting.retain(|p| *p != closed.path);
        if self.session.active.as_ref() == Some(&closed.path) {
            self.session.active = current.map(|t| t.path.clone());
        }
        if previous == self.last_active && previous == Some(closed.id) {
            self.last_active = current.map(|t| t.id);
        }
    }

    fn finished(
        &mut self,
        tabs: &mut [Tab],
        active: &mut usize,
        path: &Path,
        previous: Option<u64>,
    ) {
        let restored = self.session.files.iter().any(|p| p == path);
        let loaded = tabs.iter().any(|t| t.path == path);
        if loaded && !restored {
            self.session.active = Some(path.to_path_buf());
        } else if previous != self.last_active
            && let Some(tab) = tabs.iter().find(|t| Some(t.id) == previous)
        {
            self.session.active = Some(tab.path.clone());
        }
        if !loaded {
            self.session.files.retain(|p| p != path);
            if self.session.active.as_deref() == Some(path) {
                self.session.active = tabs
                    .iter()
                    .find(|t| Some(t.id) == previous)
                    .map(|t| t.path.clone());
            }
        }
        tabs.sort_by_key(|t| {
            self.session
                .files
                .iter()
                .position(|p| *p == t.path)
                .unwrap_or(usize::MAX)
        });
        *active = self
            .session
            .active
            .as_ref()
            .and_then(|p| tabs.iter().position(|t| t.path == *p))
            .or_else(|| tabs.iter().position(|t| Some(t.id) == previous))
            .unwrap_or(0);
        self.last_active = tabs.get(*active).map(|t| t.id);
    }
}

impl App {
    pub(crate) fn restore_workspace(&mut self, mut saved: Session, files: Vec<PathBuf>) {
        saved.limit();
        let mut session = Session::default();
        for path in &saved.files {
            session.push(path);
        }
        session.active = saved.active.map(|p| absolute(&p));
        for path in files {
            session.push(&path);
            session.active = Some(absolute(&path));
        }
        session.active = session.active.or_else(|| session.files.first().cloned());
        self.restoring = (!session.is_empty()).then(|| Restore::new(session));
        self.pump_workspace();
    }

    fn pump_workspace(&mut self) {
        while self.loading.len() < MAX_LOADING {
            let Some(path) = self.restoring.as_mut().and_then(|r| r.waiting.pop_front()) else {
                break;
            };
            if !self.tabs.iter().any(|t| t.path == path) {
                self.open(path);
            }
        }
        if let Some(restore) = &self.restoring
            && restore.waiting.is_empty()
            && !self
                .loading
                .iter()
                .any(|p| restore.session.files.contains(p))
        {
            self.restoring = None;
        }
    }

    pub(crate) fn workspace_finished(&mut self, path: &Path, previous: Option<u64>) {
        if let Some(restore) = &mut self.restoring {
            restore.finished(&mut self.tabs, &mut self.active, path, previous);
            self.pump_workspace();
        }
    }

    pub(crate) fn workspace_closed(&mut self, closed: &Tab, previous: Option<u64>) {
        if let Some(restore) = &mut self.restoring {
            restore.closed(closed, previous, self.tabs.get(self.active));
        }
    }

    /// Compare worker-resolved identities, never lexically rewrite a symlink's `..`.
    pub(crate) fn reuse_loaded_file(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        real: Option<&Path>,
    ) -> bool {
        let Some(i) = real.and_then(|p| self.tabs.iter().position(|t| t.real_path == p)) else {
            return false;
        };
        let previous = self.tabs.get(self.active).map(|t| t.id);
        let id = self.tabs[i].id;
        let kept = self.tabs[i].path.clone();
        if let Some(restore) = &mut self.restoring {
            restore.alias(path, &kept);
        }
        self.active = i;
        self.workspace_finished(path, previous);
        if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
            self.link_loaded(ctx, path, i);
        }
        true
    }
}

#[cfg(test)]
#[path = "session_review_tests.rs"]
mod review_tests;
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "session_store_tests.rs"]
mod store_tests;
#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
