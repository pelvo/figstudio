//! Small state and decision helpers for the workers: poll results, report-once flags, pending
//! loads, the open decision. Kept apart from egui so each rule has a plain unit test.

use std::path::PathBuf;

/// Result of polling a worker channel. A disconnected channel is an error, not an empty queue:
/// the worker thread is gone, so no more results will ever arrive.
#[derive(Debug, PartialEq, Eq)]
pub enum Poll<T> {
    Msg(T),
    Empty,
    Disconnected,
}

/// Maps one `try_recv` result onto `Poll`.
pub fn classify<T>(r: Result<T, std::sync::mpsc::TryRecvError>) -> Poll<T> {
    match r {
        Ok(v) => Poll::Msg(v),
        Err(std::sync::mpsc::TryRecvError::Empty) => Poll::Empty,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Poll::Disconnected,
    }
}

/// A condition that is reported once, however often it is checked.
#[derive(Default)]
pub struct Once(bool);

impl Once {
    /// True the first time only.
    pub fn first_time(&mut self) -> bool {
        !std::mem::replace(&mut self.0, true)
    }
}

/// Files whose load is in flight. A second open of the same path is ignored while it is pending.
#[derive(Default)]
pub struct Pending(Vec<PathBuf>);

impl Pending {
    /// Records `path` as loading. False when it already was, so the caller must not start a load.
    pub fn begin(&mut self, path: &PathBuf) -> bool {
        if self.0.contains(path) {
            return false;
        }
        self.0.push(path.clone());
        true
    }

    /// Clears `path` once its load finished, failed or was dropped.
    pub fn finish(&mut self, path: &PathBuf) {
        self.0.retain(|p| p != path);
    }

    pub fn iter(&self) -> impl Iterator<Item = &PathBuf> {
        self.0.iter()
    }

    /// How many loads are in flight (the browser's status report).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// What `open` does with a path: focus an open tab, start a load, or do nothing because the
/// file is already loading. Kept apart from egui so the decision can be tested directly.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenAction {
    Focus(usize),
    Load,
    Ignore,
}

pub fn open_action(open_tabs: &[&PathBuf], loading: &mut Pending, path: &PathBuf) -> OpenAction {
    if let Some(i) = open_tabs.iter().position(|p| *p == path) {
        return OpenAction::Focus(i);
    }
    if loading.begin(path) {
        OpenAction::Load
    } else {
        OpenAction::Ignore
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{TryRecvError, channel};

    #[test]
    fn a_disconnected_worker_is_not_an_empty_queue() {
        let (tx, rx) = channel::<u32>();
        assert_eq!(classify(rx.try_recv()), Poll::Empty);
        drop(tx);
        assert_eq!(classify(rx.try_recv()), Poll::Disconnected);
        assert_eq!(classify::<u32>(Err(TryRecvError::Empty)), Poll::Empty);
    }

    #[test]
    fn a_second_open_of_a_pending_file_is_ignored_until_it_finishes() {
        let mut p = Pending::default();
        let a = PathBuf::from("/x/a.fig");
        assert!(p.begin(&a), "first open starts a load");
        assert!(
            !p.begin(&a),
            "second open of the same file must not start another"
        );
        p.finish(&a);
        assert_eq!(p.iter().count(), 0);
        assert!(p.begin(&a), "after the load finished, a retry starts again");
    }

    #[test]
    fn a_dead_worker_is_reported_once_not_every_frame() {
        let mut once = Once::default();
        assert!(once.first_time(), "first sighting is reported");
        assert!(!once.first_time(), "later frames stay quiet");
        assert!(!once.first_time());
    }

    #[test]
    fn opening_a_file_twice_starts_one_load_and_focuses_an_open_tab() {
        let mut loading = Pending::default();
        let path = PathBuf::from("/virtual/drop.fig");
        assert_eq!(open_action(&[], &mut loading, &path), OpenAction::Load);
        assert_eq!(open_action(&[], &mut loading, &path), OpenAction::Ignore);
        loading.finish(&path);
        assert_eq!(
            open_action(&[], &mut loading, &path),
            OpenAction::Load,
            "failure permits retry"
        );
        loading.finish(&path);
        assert_eq!(
            open_action(&[&path], &mut loading, &path),
            OpenAction::Focus(0)
        );

        let a = PathBuf::from("/x/a.fig");
        let b = PathBuf::from("/x/b.fig");
        assert_eq!(open_action(&[], &mut loading, &a), OpenAction::Load);
        assert_eq!(open_action(&[], &mut loading, &a), OpenAction::Ignore);
        loading.finish(&a);
        assert_eq!(
            open_action(&[&a, &b], &mut loading, &b),
            OpenAction::Focus(1)
        );
    }
}
