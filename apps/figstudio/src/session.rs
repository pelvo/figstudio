//! Desktop workspace persistence. Documents still open on workers; no file contents are saved.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{App, tab::Tab};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Session {
    files: Vec<PathBuf>,
    active: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct Restore {
    session: Session,
    /// Distinguishes our automatic focus from a space selected while files were loading.
    last_active: Option<u64>,
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

impl Session {
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
                if app.tabs.iter().any(|t| t.path == *path) || app.loading.iter().any(|p| p == path)
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
            .filter(|p| session.files.contains(p));
        session
    }
}

impl Restore {
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
    pub(crate) fn restore_workspace(&mut self, saved: Session, files: Vec<PathBuf>) {
        let mut session = Session::default();
        for path in &saved.files {
            session.push(path);
        }
        session.active = saved.active.map(|p| absolute(&p));
        for path in files {
            session.push(&path);
            session.active = Some(absolute(&path));
        }
        for path in &session.files {
            self.open(path.clone());
        }
        self.restoring = (!session.files.is_empty()).then_some(Restore {
            session,
            last_active: None,
        });
    }

    pub(crate) fn workspace_finished(&mut self, path: &Path, previous: Option<u64>) {
        let Some(restore) = self.restoring.as_mut() else {
            return;
        };
        restore.finished(&mut self.tabs, &mut self.active, path, previous);
        if !self
            .loading
            .iter()
            .any(|p| restore.session.files.contains(p))
        {
            self.restoring = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn workspace_order_is_independent_of_decode_order_and_respects_a_new_focus() {
        let h = crate::ui_tests::harness();
        let doc = Arc::clone(&h.state().tabs[0].doc);
        let paths: Vec<_> = ["first.fig", "second.fig", "third.fig"]
            .map(PathBuf::from)
            .into();
        for manually_selected in [false, true] {
            let mut restore = Restore {
                session: Session {
                    files: paths.clone(),
                    active: Some(paths[1].clone()),
                },
                last_active: None,
            };
            let mut tabs = Vec::new();
            let mut active = 0;
            for i in [2, 0, 1] {
                let previous = tabs.get(active).map(|t: &Tab| t.id);
                tabs.push(Tab::new(
                    i as u64,
                    paths[i].clone(),
                    Arc::clone(&doc),
                    None,
                    Duration::ZERO,
                ));
                active = tabs.len() - 1;
                restore.finished(&mut tabs, &mut active, &paths[i], previous);
                if i == 0 && manually_selected {
                    active = 0;
                }
            }
            assert_eq!(
                tabs.iter().map(|t| &t.path).collect::<Vec<_>>(),
                paths.iter().collect::<Vec<_>>()
            );
            assert_eq!(tabs[active].path, paths[usize::from(!manually_selected)]);
        }
    }
}
