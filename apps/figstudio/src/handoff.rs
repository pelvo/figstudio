//! One figstudio per person: a launch with files hands them to the figstudio already
//! running, which opens them in its window, and exits. The Windows jump list starts
//! figstudio with a file, so its items open in the window the person is using. A launch
//! without files starts a window as before (two windows then share Recents).
//!
//! The running figstudio holds a lock on `<config>/figstudio/instance.lock`, taken before
//! its window opens; the system drops it when the process ends, crash or not. A launch that
//! finds it held writes its files as a request in `<config>/figstudio/inbox/`, waits for the
//! running one to be up (its `ready` mark), then a moment for its window to take the
//! request. Taking is one rename, and a launch that gives up removes its request: only one
//! of the two can succeed, so the files open in one window only. The folder is the
//! person's own; on Unix this makes it `0700` and refuses one that is a link elsewhere.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// How long a launch waits for the running figstudio's window to take its files.
pub(crate) const WAIT: Duration = Duration::from_secs(2);
/// How long a launch waits for a figstudio still starting (its window, its GPU).
pub(crate) const STARTING: Duration = Duration::from_secs(20);
/// How often the running figstudio looks in its inbox.
const LOOK: Duration = Duration::from_millis(150);
/// How often a launch looks whether its request was taken.
const CHECK: Duration = Duration::from_millis(20);
/// The largest request: a launch keeps files that would make a larger one.
const MAX_REQUEST: u64 = 1 << 20;
/// In the inbox while the running figstudio's window takes requests; it holds that
/// figstudio's id, as `HOLDER` does, so a mark left by one that ended is not taken as up.
const READY: &str = "ready";
/// In the inbox: the id of the figstudio holding the lock.
const HOLDER: &str = "holder";
/// How long a launch keeps trying to take back a request it gave up on.
const GIVE_UP: Duration = Duration::from_secs(5);

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
    /// No lock could be had (no config folder, a file system without locks, a folder that
    /// is not the person's own): on its own.
    Alone,
}

/// The lock of the running figstudio, and its inbox, before its window is up.
#[derive(Debug)]
pub(crate) struct Instance {
    _lock: File,
    inbox: PathBuf,
    /// Written to `READY` once the window takes requests.
    id: String,
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

/// Makes `dir` (and its parents) and checks it is the person's own: on Unix a real
/// folder, not a link, readable by its owner only.
fn own_folder(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if fs::symlink_metadata(dir)?.file_type().is_symlink() {
            return Err(std::io::Error::other("a link, not a folder"));
        }
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Takes the lock in `dir`, or finds another figstudio holding it.
pub(crate) fn claim(dir: &Path) -> Start {
    let inbox = dir.join("inbox");
    if let Err(e) = own_folder(dir).and_then(|()| own_folder(&inbox)) {
        tracing::warn!("single instance: {e}");
        return Start::Alone;
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let Ok(lock) = options.open(dir.join("instance.lock")) else {
        return Start::Alone;
    };
    match lock.try_lock() {
        Ok(()) => {
            let id = unique();
            recover(&inbox);
            if let Err(e) = fs::write(inbox.join(HOLDER), &id) {
                tracing::warn!("single instance: {e}");
            }
            Start::First(Instance {
                _lock: lock,
                inbox,
                id,
            })
        }
        Err(TryLockError::WouldBlock) => Start::Running(inbox),
        Err(TryLockError::Error(e)) => {
            tracing::warn!("single instance: {e}");
            Start::Alone
        }
    }
}

/// What a figstudio that ended left in `inbox` (this one holds the lock now): its `ready`
/// mark and half-written requests go; requests it took but never opened are waiting
/// again (their launches counted them as taken and left).
fn recover(inbox: &Path) {
    let _ = fs::remove_file(inbox.join(READY));
    let Ok(entries) = fs::read_dir(inbox) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        match path.extension().and_then(|e| e.to_str()) {
            Some("part" | "withdrawn") => {
                let _ = fs::remove_file(&path);
            }
            Some("taken") => {
                let _ = fs::rename(&path, path.with_extension("json"));
            }
            _ => {}
        }
    }
}

/// Hands `files` to the figstudio whose inbox is `inbox`: waits up to `starting` for it to
/// be up, then up to `wait` for its window to take them. True when it took them (this
/// process is not needed); false when there are none, they cannot be written down, or they
/// were not known to be taken (they are this process's to open).
pub(crate) fn hand_over(
    inbox: &Path,
    files: &[PathBuf],
    wait: Duration,
    starting: Duration,
) -> bool {
    if files.is_empty() {
        return false;
    }
    // Absolute here: the running figstudio was started somewhere else. A path that cannot
    // be made absolute, or written as text, stays with this launch, as do files too many
    // for one request.
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
    if text.len() as u64 > MAX_REQUEST {
        return false;
    }
    let id = unique();
    let partial = inbox.join(format!("{id}.part"));
    let request = inbox.join(format!("{id}.json"));
    // Whole or not at all: the running figstudio reads only `.json`.
    if fs::write(&partial, text).is_err() || fs::rename(&partial, &request).is_err() {
        let _ = fs::remove_file(&partial);
        return false;
    }
    // A figstudio still starting takes nothing yet: wait for its window (its own mark, not
    // one left by a figstudio that ended), then for the window to take the request.
    let up = Instant::now() + starting;
    let is_up = || {
        let read = |name| fs::read_to_string(inbox.join(name));
        matches!((read(READY), read(HOLDER)), (Ok(ready), Ok(holder)) if ready == holder)
    };
    while !is_up() && Instant::now() < up {
        std::thread::sleep(CHECK);
    }
    // Before the window takes the files, while this launch still may give the foreground.
    let holder = fs::read_to_string(inbox.join(HOLDER)).ok();
    let_come_forward(holder.as_deref().and_then(holder_pid));
    let until = Instant::now() + wait;
    while Instant::now() < until {
        // Only a request known to be gone was taken; one that cannot be looked at is not.
        if matches!(fs::exists(&request), Ok(false)) {
            return true;
        }
        std::thread::sleep(CHECK);
    }
    // Not taken in time: take it back, by deleting it or moving it where the running one
    // does not look. Only that makes the files this launch's; gone means it was taken after
    // all. Other errors (a scanner holding the new file a moment) are tried again.
    let withdrawn = inbox.join(format!("{id}.withdrawn"));
    let until = Instant::now() + GIVE_UP;
    loop {
        match fs::remove_file(&request) {
            Ok(()) => return false,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => {}
        }
        match fs::rename(&request, &withdrawn) {
            Ok(()) => {
                let _ = fs::remove_file(&withdrawn);
                return false;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => {}
        }
        if Instant::now() >= until {
            tracing::warn!("single instance: a request could not be taken back");
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The process of a lock holder's id (`unique`), so a launch can let its window come
/// forward.
fn holder_pid(id: &str) -> Option<u32> {
    id.split_once('-')?.0.parse().ok()
}

/// Lets the running figstudio's window come forward when it opens the files: Windows gives
/// the foreground only to the process the person just started, which is this launch, and
/// this passes it on. Elsewhere the window comes forward by itself.
#[cfg(windows)]
fn let_come_forward(holder: Option<u32>) {
    use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    #[expect(
        unsafe_code,
        reason = "Windows' foreground permission is a Win32 call (approved by the owner)"
    )]
    // SAFETY: a plain Win32 call that takes a process id (or `ASFW_ANY`) by value and reads
    // or writes no memory of this process; any failure is reported as an error.
    let allowed = unsafe { AllowSetForegroundWindow(holder.unwrap_or(ASFW_ANY)) };
    if let Err(e) = allowed {
        tracing::warn!("single instance: the window may not come forward: {e}");
    }
}

#[cfg(not(windows))]
fn let_come_forward(_holder: Option<u32>) {}

/// An id no other request or figstudio has: the process and the time.
fn unique() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    )
}

impl Instance {
    /// From now on, `wake` is called while a later launch's request waits; the window then
    /// takes it with [`Inbox::take`]. The lock goes with the inbox.
    pub(crate) fn watch(self, wake: impl Fn() + Send + 'static) -> Inbox {
        if let Err(e) = fs::write(self.inbox.join(READY), &self.id) {
            tracing::warn!("single instance: {e}");
        }
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
        // Launches write at most `MAX_REQUEST`, whole, so this reads all of it. A read that
        // fails is tried again a few times (a scanner may hold the new file a moment).
        let read = || {
            File::open(&mine).ok().and_then(|f| {
                use std::io::Read as _;
                let mut text = String::new();
                f.take(MAX_REQUEST).read_to_string(&mut text).ok()?;
                serde_json::from_str::<Request>(&text).ok()
            })
        };
        let request = (0..5).find_map(|attempt| {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(20));
            }
            read()
        });
        match request {
            Some(request) => {
                let _ = fs::remove_file(&mine);
                taken.push(request.open);
            }
            // Not a launch's request: kept aside, not read again.
            None => tracing::warn!("single instance: a request was not readable"),
        }
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The requests in `inbox` (written, waiting, taken or withdrawn), not the marks.
    fn requests(inbox: &Path) -> usize {
        fs::read_dir(inbox)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some())
            .count()
    }

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
        let launch = std::thread::spawn(move || hand_over(&to, &files, WAIT, STARTING));
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
            Duration::from_millis(100),
            Duration::from_millis(100)
        ));
        assert_eq!(requests(&inbox), 0);
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
            Duration::from_millis(400),
            STARTING
        ));
    }

    #[test]
    fn a_launch_waits_for_a_figstudio_still_starting() {
        let dir = tempfile::tempdir().unwrap();
        // Holds the lock, but its window (and its inbox watch) is not up yet.
        let starting = first(dir.path());
        let to = inbox_of(dir.path());
        let launch = std::thread::spawn(move || {
            hand_over(
                &to,
                &[PathBuf::from("/a.fig")],
                Duration::from_millis(300),
                Duration::from_secs(5),
            )
        });
        // Longer than the wait: the window took its time.
        std::thread::sleep(Duration::from_millis(600));
        let inbox = starting.watch(|| {});
        let until = Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while got.is_empty() && Instant::now() < until {
            got = inbox.take();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(got, [PathBuf::from("/a.fig")]);
        assert!(
            launch.join().unwrap(),
            "taken, so the launch does not open them too"
        );
    }

    #[test]
    fn a_request_too_big_to_read_stays_with_the_launch() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = first(dir.path()).watch(|| {});
        let to = inbox_of(dir.path());
        let long = PathBuf::from(format!("/{}.fig", "x".repeat(MAX_REQUEST as usize)));
        let launch = std::thread::spawn(move || {
            hand_over(&to, &[long], Duration::from_millis(500), STARTING)
        });
        // The window takes what it can read.
        let until = Instant::now() + Duration::from_secs(1);
        while Instant::now() < until {
            assert!(inbox.take().is_empty());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !launch.join().unwrap(),
            "kept by the launch, not taken and lost"
        );
    }

    #[test]
    fn what_a_crash_left_in_the_inbox_is_cleared_or_opened() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = dir.path().join("inbox");
        fs::create_dir_all(&inbox).unwrap();
        // A launch killed while writing; a window killed after taking a request.
        fs::write(inbox.join("1-1.part"), "{").unwrap();
        fs::write(inbox.join("2-2.taken"), r#"{"open":["/left.fig"]}"#).unwrap();
        let inbox = first(dir.path()).watch(|| {});
        assert_eq!(
            inbox.take(),
            [PathBuf::from("/left.fig")],
            "its launch had left"
        );
        assert_eq!(requests(&dir.path().join("inbox")), 0, "nothing left over");
    }

    /// A stat that fails is not the request taken (Unix: a folder that cannot be searched).
    #[cfg(unix)]
    #[test]
    fn a_request_that_cannot_be_looked_at_is_not_counted_as_taken() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let _inbox = first(dir.path()).watch(|| {});
        let to = inbox_of(dir.path());
        let shut = to.clone();
        let block = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            fs::set_permissions(&shut, fs::Permissions::from_mode(0o600)).unwrap();
        });
        let handed = hand_over(
            &to,
            &[PathBuf::from("/a.fig")],
            Duration::from_millis(400),
            STARTING,
        );
        block.join().unwrap();
        fs::set_permissions(&to, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!handed, "not known to be taken: the launch keeps the files");
    }

    #[cfg(unix)]
    #[test]
    fn the_folder_is_the_persons_own() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("figstudio");
        let _held = first(&folder);
        for path in [&folder, &folder.join("inbox")] {
            let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", path.display());
        }
        // A folder that is a link elsewhere is not used.
        let elsewhere = tempfile::tempdir().unwrap();
        let linked = dir.path().join("linked");
        std::os::unix::fs::symlink(elsewhere.path(), &linked).unwrap();
        assert!(matches!(claim(&linked), Start::Alone));
    }

    #[test]
    fn a_ready_mark_left_by_an_ended_figstudio_is_not_taken_as_up() {
        let dir = tempfile::tempdir().unwrap();
        let starting = first(dir.path());
        // Left by the figstudio before (on Windows it may not be deletable at once).
        fs::write(dir.path().join("inbox").join(READY), "an ended one").unwrap();
        let to = inbox_of(dir.path());
        let launch = std::thread::spawn(move || {
            hand_over(
                &to,
                &[PathBuf::from("/a.fig")],
                Duration::from_millis(300),
                Duration::from_secs(5),
            )
        });
        std::thread::sleep(Duration::from_millis(600));
        let inbox = starting.watch(|| {});
        let until = Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while got.is_empty() && Instant::now() < until {
            got = inbox.take();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(got, [PathBuf::from("/a.fig")]);
        assert!(
            launch.join().unwrap(),
            "it waited for this figstudio, so it did not open them too"
        );
    }

    /// A request that cannot be taken back at once is still taken back before the launch
    /// opens the files itself, so they never open twice (Unix: a folder shut a while).
    #[cfg(unix)]
    #[test]
    fn a_launch_opens_its_files_only_once_its_request_is_withdrawn() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let _inbox = first(dir.path()).watch(|| {});
        let to = inbox_of(dir.path());
        let shut = to.clone();
        let block = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            fs::set_permissions(&shut, fs::Permissions::from_mode(0o600)).unwrap();
            // Open again well after the launch's wait.
            std::thread::sleep(Duration::from_millis(1200));
            fs::set_permissions(&shut, fs::Permissions::from_mode(0o700)).unwrap();
        });
        let handed = hand_over(
            &to,
            &[PathBuf::from("/a.fig")],
            Duration::from_millis(300),
            STARTING,
        );
        block.join().unwrap();
        assert!(!handed);
        let left: Vec<_> = fs::read_dir(&to)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .collect();
        assert!(
            left.is_empty(),
            "no request left for the running one to open too"
        );
    }

    /// A request that cannot be read the moment it is taken is read again (a scanner may
    /// hold a new file); Unix: unreadable for a moment.
    #[cfg(unix)]
    #[test]
    fn a_request_unreadable_for_a_moment_is_read_again() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let inbox = first(dir.path()).watch(|| {});
        let request = dir.path().join("inbox").join("9-9.json");
        fs::write(&request, r#"{"open":["/later.fig"]}"#).unwrap();
        fs::set_permissions(&request, fs::Permissions::from_mode(0o000)).unwrap();
        let taken = dir.path().join("inbox").join("9-9.taken");
        let reopen = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            fs::set_permissions(&taken, fs::Permissions::from_mode(0o600)).unwrap();
        });
        assert_eq!(inbox.take(), [PathBuf::from("/later.fig")]);
        reopen.join().unwrap();
    }

    #[test]
    fn the_holders_process_is_read_from_its_id() {
        assert_eq!(holder_pid(&unique()), Some(std::process::id()));
        assert_eq!(holder_pid("not an id"), None);
        assert_eq!(holder_pid(""), None);
    }

    #[test]
    fn a_launch_without_files_starts_its_own_window() {
        let dir = tempfile::tempdir().unwrap();
        let _held = first(dir.path());
        assert!(!hand_over(&inbox_of(dir.path()), &[], WAIT, STARTING));
    }
}
