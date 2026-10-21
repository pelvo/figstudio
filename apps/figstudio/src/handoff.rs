//! One figstudio per person: a launch with files hands them to the figstudio already
//! running, which opens them in its window, and exits. The Windows jump list starts
//! figstudio with a file, so its items open in the window the person is using, and only one
//! process keeps the Recents list. A launch without files starts as before.
//!
//! The running figstudio listens on a local socket of the user's: a named pipe with the
//! user's name on Windows, a socket file in the user's figstudio config folder elsewhere
//! (not the shared `/tmp`, where another account could take the name). A launch sends one JSON line, `{"open":[paths]}`, and
//! waits a moment for `ok`; without one it opens the files itself.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, ListenerOptions, Name, Stream, prelude::*,
};
use serde::{Deserialize, Serialize};

/// How long a launch waits for the running figstudio, and how long that one waits for a
/// launch's line.
const WAIT: Duration = Duration::from_secs(2);
/// The longest line read (a few thousand paths).
const MAX_LINE: u64 = 1 << 20;

#[derive(Serialize, Deserialize)]
struct Message {
    open: Vec<PathBuf>,
}

/// The socket for this user: a path (with a separator) is a socket file, anything else a
/// name in the system's namespace (a named pipe on Windows).
pub(crate) fn name() -> String {
    #[cfg(unix)]
    if let Some(dir) = dirs::config_dir() {
        let dir = dir.join("figstudio");
        if std::fs::create_dir_all(&dir).is_ok() {
            return dir.join("instance.sock").to_string_lossy().into_owned();
        }
    }
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let user: String = user
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(32)
        .collect();
    format!("figstudio-{user}.sock")
}

/// Hands `files` to the figstudio listening on `name`. True when it took them (this
/// process is not needed); false when there are none, nothing listens, or it did not answer.
pub(crate) fn hand_over(name: &str, files: &[PathBuf]) -> bool {
    if files.is_empty() {
        return false;
    }
    // Absolute here: the running figstudio was started somewhere else.
    let open = files
        .iter()
        .map(|f| std::path::absolute(f).unwrap_or_else(|_| f.clone()))
        .collect();
    let Ok(mut line) = serde_json::to_string(&Message { open }) else {
        return false;
    };
    line.push('\n');
    let Ok(name) = socket(name) else {
        return false;
    };
    let Ok(stream) = Stream::connect(name) else {
        return false;
    };
    if stream.set_recv_timeout(Some(WAIT)).is_err() || stream.set_send_timeout(Some(WAIT)).is_err()
    {
        return false;
    }
    let mut stream = BufReader::new(stream);
    if stream.get_mut().write_all(line.as_bytes()).is_err() {
        return false;
    }
    let mut answer = String::new();
    (&mut stream).take(16).read_line(&mut answer).is_ok() && answer.trim() == "ok"
}

/// `name` as a socket address (see [`name`]).
fn socket(name: &str) -> std::io::Result<Name<'_>> {
    if name.contains(std::path::MAIN_SEPARATOR) {
        name.to_fs_name::<GenericFilePath>()
    } else {
        name.to_ns_name::<GenericNamespaced>()
    }
}

/// Listens on `name` for files handed over by later launches; `wake` is called after each
/// batch arrives. `None` when another figstudio listens already.
pub(crate) fn listen(
    name: &str,
    wake: impl Fn() + Send + 'static,
) -> Option<Receiver<Vec<PathBuf>>> {
    let socket = || socket(name);
    let listener = match ListenerOptions::new().name(socket().ok()?).create_sync() {
        Ok(listener) => listener,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            // Taken: by a figstudio that answers (it keeps it), or left by one that crashed.
            if Stream::connect(socket().ok()?).is_ok() {
                return None;
            }
            match ListenerOptions::new()
                .name(socket().ok()?)
                .try_overwrite(true)
                .create_sync()
            {
                Ok(listener) => listener,
                Err(e) => {
                    tracing::warn!("single instance: {e}");
                    return None;
                }
            }
        }
        Err(e) => {
            tracing::warn!("single instance: {e}");
            return None;
        }
    };
    let (tx, rx) = channel();
    let accept = move || {
        for conn in listener.incoming().filter_map(Result::ok) {
            if conn.set_recv_timeout(Some(WAIT)).is_err() {
                continue;
            }
            let mut conn = BufReader::new(conn);
            let mut line = String::new();
            if (&mut conn).take(MAX_LINE).read_line(&mut line).is_err() {
                continue;
            }
            // Not a launch's line (one checking that this figstudio answers): nothing to do.
            let Ok(message) = serde_json::from_str::<Message>(&line) else {
                continue;
            };
            if tx.send(message.open).is_err() {
                return;
            }
            wake();
            let _ = conn.get_mut().write_all(b"ok\n");
        }
    };
    std::thread::Builder::new()
        .name("handoff".into())
        .spawn(accept)
        .map_err(|e| tracing::warn!("single instance: {e}"))
        .ok()?;
    Some(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A name no other test or figstudio uses.
    fn unique(tag: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        format!("figstudio-test-{tag}-{}-{nanos}.sock", std::process::id())
    }

    #[test]
    fn files_handed_to_a_running_figstudio_arrive_there() {
        let name = unique("arrive");
        let inbox = listen(&name, || {}).expect("nothing else listens on a fresh name");
        let files = [PathBuf::from("a.fig"), PathBuf::from("/b/c d.fig")];
        assert!(hand_over(&name, &files));
        let got = inbox
            .recv_timeout(Duration::from_secs(5))
            .expect("the files arrive");
        // Relative paths are made absolute where the launch was started.
        assert_eq!(got[0], std::path::absolute("a.fig").unwrap());
        assert_eq!(got[1], PathBuf::from("/b/c d.fig"));
    }

    #[test]
    fn with_none_running_a_launch_opens_its_files_itself() {
        assert!(!hand_over(&unique("none"), &[PathBuf::from("a.fig")]));
    }

    #[test]
    fn a_launch_without_files_starts_its_own_window() {
        let name = unique("empty");
        let inbox = listen(&name, || {}).expect("listening");
        assert!(!hand_over(&name, &[]));
        assert!(inbox.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn only_the_first_figstudio_listens() {
        let name = unique("second");
        let _first = listen(&name, || {}).expect("listening");
        assert!(
            listen(&name, || {}).is_none(),
            "the second leaves it to the first"
        );
    }

    /// A socket file left by a figstudio that crashed is taken over.
    #[cfg(unix)]
    #[test]
    fn a_socket_left_by_a_crash_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("instance.sock");
        let name = path.to_string_lossy().into_owned();
        drop(std::os::unix::net::UnixListener::bind(&path).expect("bind"));
        assert!(path.exists(), "the file stays, as after a crash");
        let inbox = listen(&name, || {}).expect("the stale name is taken over");
        assert!(hand_over(&name, &[PathBuf::from("/x.fig")]));
        assert_eq!(
            inbox.recv_timeout(Duration::from_secs(5)).unwrap(),
            [PathBuf::from("/x.fig")]
        );
    }
}
