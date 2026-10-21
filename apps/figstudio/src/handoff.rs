//! One figstudio per person: a launch with files hands them to the figstudio already
//! running, which opens them in its window, and exits. The Windows jump list starts
//! figstudio with a file, so its items open in the window the person is using. A launch
//! without files starts a window as before (two windows then share Recents).
//!
//! The running figstudio holds a lock on `<config>/figstudio/instance.lock`, taken before
//! its window opens; the system drops it when the process ends, crash or not. A launch that
//! finds it held writes its files as a request in `<config>/figstudio/inbox/` and waits a
//! moment for the running one to take it. Taking is one rename, and a launch that gives up
//! removes its request: whichever comes first wins, so the files open in one window only.
//! Both live in the person's own config folder, which other accounts cannot write.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// How long a launch waits for the running figstudio to take its files.
pub(crate) const WAIT: Duration = Duration::from_secs(2);
/// How often the running figstudio looks in its inbox.
const LOOK: Duration = Duration::from_millis(150);
/// How often a launch looks whether its request was taken.
const CHECK: Duration = Duration::from_millis(20);
/// The largest request read.
const MAX_REQUEST: u64 = 1 << 20;

#[derive(Serialize, Deserialize)]
struct Request {
    open: Vec<PathBuf>,
}

/// Where this figstudio stands among the others of this person.
#[derive(Debug)]
pub(crate) enum Start {
    /// The first: it keeps the lock and takes files from later launches.
    First(Instance),
    /// Another runs; files go to its inbox.
    Running(PathBuf),
    /// No lock could be had (no config folder, a file system without locks): on its own.
    Alone,
}

/// The lock of the running figstudio, and its inbox, before its window is up.
#[derive(Debug)]
pub(crate) struct Instance {
    _lock: File,
    inbox: PathBuf,
}

/// The running figstudio's inbox. The window takes the requests (`take`), so a hung window
/// takes none and a launch opens its own files; a thread only wakes the window.
#[derive(Debug)]
pub(crate) struct Inbox {
    _lock: File,
    dir: PathBuf,
    /// The waking thread stops once this is dropped.
    _alive: Arc<()>,
}

/// The folder for the lock and the inbox: `<config>/figstudio`.
pub(crate) fn folder() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("figstudio"))
}

/// Takes the lock in `dir`, or finds another figstudio holding it.
pub(crate) fn claim(dir: &Path) -> Start {
    let inbox = dir.join("inbox");
    if fs::create_dir_all(&inbox).is_err() {
        return Start::Alone;
    }
    let Ok(lock) = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("instance.lock"))
    else {
        return Start::Alone;
    };
    match lock.try_lock() {
        Ok(()) => Start::First(Instance { _lock: lock, inbox }),
        Err(TryLockError::WouldBlock) => Start::Running(inbox),
        Err(TryLockError::Error(e)) => {
            tracing::warn!("single instance: {e}");
            Start::Alone
        }
    }
}

/// Hands `files` to the figstudio whose inbox is `inbox`. True when it took them within
/// `wait` (this process is not needed); false when there are none, they cannot be written
/// down, or it did not take them (they are this process's to open).
pub(crate) fn hand_over(inbox: &Path, files: &[PathBuf], wait: Duration) -> bool {
    if files.is_empty() {
        return false;
    }
    // Absolute here: the running figstudio was started somewhere else. A path that cannot
    // be made absolute, or written as text, stays with this launch.
    let Ok(open) = files
        .iter()
        .map(std::path::absolute)
        .collect::<std::io::Result<Vec<_>>>()
    else {
        return false;
    };
    let Ok(text) = serde_json::to_string(&Request { open }) else {
        return false;
    };
    let id = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    );
    let partial = inbox.join(format!("{id}.part"));
    let request = inbox.join(format!("{id}.json"));
    // Whole or not at all: the running figstudio reads only `.json`.
    if fs::write(&partial, text).is_err() || fs::rename(&partial, &request).is_err() {
        let _ = fs::remove_file(&partial);
        return false;
    }
    let until = Instant::now() + wait;
    while Instant::now() < until {
        if !request.exists() {
            return true;
        }
        std::thread::sleep(CHECK);
    }
    // Not taken in time: take it back. If that fails because it is gone, it was taken.
    match fs::remove_file(&request) {
        Ok(()) => false,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

impl Instance {
    /// From now on, `wake` is called while a later launch's request waits; the window then
    /// takes it with [`Inbox::take`]. The lock goes with the inbox.
    pub(crate) fn watch(self, wake: impl Fn() + Send + 'static) -> Inbox {
        let alive = Arc::new(());
        let (dir, weak): (PathBuf, Weak<()>) = (self.inbox.clone(), Arc::downgrade(&alive));
        let look = move || {
            while weak.upgrade().is_some() {
                if waiting(&dir) {
                    wake();
                }
                std::thread::sleep(LOOK);
            }
        };
        if let Err(e) = std::thread::Builder::new()
            .name("handoff".into())
            .spawn(look)
        {
            tracing::warn!("single instance: {e}");
        }
        Inbox {
            _lock: self._lock,
            dir: self.inbox,
            _alive: alive,
        }
    }
}

impl Inbox {
    /// The files of the requests waiting, taken now (on the window's thread).
    pub(crate) fn take(&self) -> Vec<PathBuf> {
        take(&self.dir).into_iter().flatten().collect()
    }
}

/// Whether a request waits in `inbox`.
fn waiting(inbox: &Path) -> bool {
    fs::read_dir(inbox).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "json"))
    })
}

/// The requests in `inbox`, each taken by renaming it first (a launch that gave up has
/// removed its own, and only one of the two can succeed).
fn take(inbox: &Path) -> Vec<Vec<PathBuf>> {
    let Ok(entries) = fs::read_dir(inbox) else {
        return Vec::new();
    };
    let mut taken = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let mine = path.with_extension("taken");
        if fs::rename(&path, &mine).is_err() {
            continue;
        }
        let request = File::open(&mine).ok().and_then(|f| {
            use std::io::Read as _;
            let mut text = String::new();
            f.take(MAX_REQUEST).read_to_string(&mut text).ok()?;
            serde_json::from_str::<Request>(&text).ok()
        });
        let _ = fs::remove_file(&mine);
        if let Some(request) = request {
            taken.push(request.open);
        }
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(dir: &Path) -> Instance {
        match claim(dir) {
            Start::First(instance) => instance,
            other => panic!("expected the lock, got {other:?}"),
        }
    }

    fn inbox_of(dir: &Path) -> PathBuf {
        match claim(dir) {
            Start::Running(inbox) => inbox,
            other => panic!("expected another to hold it, got {other:?}"),
        }
    }

    #[test]
    fn files_handed_to_a_running_figstudio_arrive_there() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = first(dir.path()).watch(|| {});
        let to = inbox_of(dir.path());
        let files = [PathBuf::from("a.fig"), PathBuf::from("/b/c d.fig")];
        let launch = std::thread::spawn(move || hand_over(&to, &files, WAIT));
        // The window takes them.
        let until = Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while got.is_empty() && Instant::now() < until {
            got = inbox.take();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(launch.join().unwrap(), "the launch sees them taken");
        // Relative paths are made absolute where the launch was started.
        assert_eq!(got[0], std::path::absolute("a.fig").unwrap());
        assert_eq!(got[1], std::path::absolute("/b/c d.fig").unwrap());
    }

    #[test]
    fn only_the_first_figstudio_holds_the_lock_until_it_ends() {
        let dir = tempfile::tempdir().unwrap();
        let instance = first(dir.path());
        inbox_of(dir.path());
        // Ended (crash or not): the system drops the lock, nothing is left to clear.
        drop(instance);
        first(dir.path());
    }

    #[test]
    fn files_not_taken_in_time_stay_with_the_launch_and_leave_no_request() {
        let dir = tempfile::tempdir().unwrap();
        let _held = first(dir.path());
        let inbox = inbox_of(dir.path());
        // Nobody watches the inbox (the running figstudio is hung).
        assert!(!hand_over(
            &inbox,
            &[PathBuf::from("/a.fig")],
            Duration::from_millis(100)
        ));
        assert_eq!(fs::read_dir(&inbox).unwrap().count(), 0);
        // Taken after the launch gave up: nothing left to take, so it opens once.
        assert!(take(&inbox).is_empty());
    }

    #[test]
    fn files_a_hung_window_does_not_take_stay_with_the_launch() {
        let dir = tempfile::tempdir().unwrap();
        // Watched, but the window never takes (it is hung).
        let _inbox = first(dir.path()).watch(|| {});
        let inbox = inbox_of(dir.path());
        assert!(!hand_over(
            &inbox,
            &[PathBuf::from("/a.fig")],
            Duration::from_millis(400)
        ));
    }

    #[test]
    fn a_launch_without_files_starts_its_own_window() {
        let dir = tempfile::tempdir().unwrap();
        let _held = first(dir.path());
        assert!(!hand_over(&inbox_of(dir.path()), &[], WAIT));
    }
}
