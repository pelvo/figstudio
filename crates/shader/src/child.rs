//! Shader runs in a child process: the same program started with [`WORKER_ARG`], which keeps a
//! GPU device and runs one shader at a time for its parent. A run past its deadline kills the
//! child, which stops what nothing inside the process can (a built-in looping in native code),
//! and a child that runs out of memory takes only itself down. The next run starts a new one.
//!
//! Messages on the child's stdin and stdout are frames: a little-endian `u32` length and a
//! JSON header, then a `u64` length and raw bytes (the effect's input, or the output pixels).

use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};

use crate::{ShaderError, js};

/// The argument that makes a figstudio program a shader worker (see [`serve_if_worker`]).
pub const WORKER_ARG: &str = "--figstudio-shader-worker";

/// Longest a new worker may take to say it is ready (it starts before its GPU does).
const READY_WITHIN: Duration = Duration::from_secs(10);

/// Largest JSON header of a frame.
const MAX_HEADER: u32 = 4 * 1024 * 1024;
/// Largest payload of a frame: an output or input of the largest size, 2048 × 2048 RGBA8.
const MAX_PAYLOAD: u64 = 2048 * 2048 * 4;

static PROGRAM: OnceLock<PathBuf> = OnceLock::new();

/// Runs shaders with `program` (which must call [`serve_if_worker`] first thing) instead of
/// this program; for tests, whose own binary cannot be a worker.
pub fn set_worker_program(program: PathBuf) {
    let _ = PROGRAM.set(program);
}

/// When this process was started as a shader worker, serves its parent until stdin closes
/// and returns `true`; the caller then exits. Call it first thing in `main`.
pub fn serve_if_worker() -> bool {
    if std::env::args_os().nth(1).is_none_or(|a| a != WORKER_ARG) {
        return false;
    }
    serve(
        &mut BufReader::new(io::stdin().lock()),
        &mut BufWriter::new(io::stdout().lock()),
    );
    true
}

/// The worker's loop: says it is ready, then one job in, one reply out.
fn serve(input: &mut impl Read, output: &mut impl Write) {
    if write_frame(output, &json!({"ready": true}), &[]).is_err() {
        return;
    }
    while let Ok((job, pixels)) = read_frame(input) {
        let reply = run_job(&job, (!pixels.is_empty()).then_some(pixels.as_slice()));
        let written = match reply {
            Ok(rgba) => write_frame(output, &json!({"ok": true}), &rgba),
            Err(e) => write_frame(output, &json!({"ok": false, "error": e.to_wire()}), &[]),
        };
        if written.is_err() {
            return;
        }
    }
}

fn run_job(job: &Value, input: Option<&[u8]>) -> Result<Vec<u8>, ShaderError> {
    let js = job["js"].as_str().unwrap_or_default();
    let size = |k: &str| {
        job[k]
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .unwrap_or(1)
    };
    let frame = js::Frame {
        width: size("width"),
        height: size("height"),
        time_ms: job["time_ms"].as_f64().unwrap_or(0.0),
        input,
    };
    js::run(js, &job["params"], &frame)
}

/// The running worker, and the replies its stdout reader passes on.
struct Worker {
    process: Child,
    stdin: BufWriter<ChildStdin>,
    replies: Receiver<io::Result<(Value, Vec<u8>)>>,
}

impl Worker {
    fn start() -> Result<Worker, ShaderError> {
        let program = match PROGRAM.get() {
            Some(p) => p.clone(),
            None => std::env::current_exe()
                .map_err(|e| ShaderError::script(format!("cannot find this program: {e}")))?,
        };
        let mut process = Command::new(program)
            .arg(WORKER_ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| ShaderError::script(format!("cannot start the shader process: {e}")))?;
        let (Some(stdin), Some(stdout)) = (process.stdin.take(), process.stdout.take()) else {
            let _ = process.kill();
            return Err(ShaderError::script("the shader process has no pipes"));
        };
        let (send, replies) = channel();
        std::thread::Builder::new()
            .name("shader-replies".into())
            .spawn(move || {
                let mut stdout = BufReader::new(stdout);
                loop {
                    let reply = read_frame(&mut stdout);
                    let done = reply.is_err();
                    if send.send(reply).is_err() || done {
                        return;
                    }
                }
            })
            .map_err(|e| ShaderError::script(format!("cannot start the shader reader: {e}")))?;
        let worker = Worker {
            process,
            stdin: BufWriter::new(stdin),
            replies,
        };
        match worker.replies.recv_timeout(READY_WITHIN) {
            Ok(Ok((hello, _))) if hello["ready"].as_bool() == Some(true) => Ok(worker),
            _ => {
                worker.stop();
                NO_WORKER.store(true, Ordering::Relaxed);
                Err(ShaderError::script(
                    "this program cannot run shader code (its shader process did not start)",
                ))
            }
        }
    }

    fn stop(mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// The running worker's process ID, if one is running (tests check a killed one is gone).
#[doc(hidden)]
pub fn worker_pid() -> Option<u32> {
    let slot = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().map(|w| w.process.id())
}

/// What a run in the worker gave back.
pub(crate) enum Outcome {
    Done(Result<Vec<u8>, ShaderError>),
    /// Past the deadline: the worker was killed.
    TimedOut,
}

/// One worker for the whole process: runs take turns.
static WORKER: Mutex<Option<Worker>> = Mutex::new(None);
/// A started program did not say it was ready: it cannot serve as a worker (a test binary,
/// for example), so no more are started.
static NO_WORKER: AtomicBool = AtomicBool::new(false);

/// Runs `js` in the worker (started on first use) and waits at most `deadline`.
pub(crate) fn run(
    js: &str,
    params: &Value,
    (width, height, time_ms): (u16, u16, f64),
    input: Option<&[u8]>,
    deadline: Duration,
) -> Outcome {
    let mut slot = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    if NO_WORKER.load(Ordering::Relaxed) {
        return Outcome::Done(Err(ShaderError::script(
            "this program cannot run shader code (its shader process did not start before; \
             not started again)",
        )));
    }
    let worker = match slot.take() {
        Some(w) => w,
        None => match Worker::start() {
            Ok(w) => w,
            Err(e) => return Outcome::Done(Err(e)),
        },
    };
    let mut worker = worker;
    let job = json!({"js": js, "params": params, "width": width, "height": height,
        "time_ms": time_ms});
    if write_frame(&mut worker.stdin, &job, input.unwrap_or_default()).is_err() {
        worker.stop();
        return Outcome::Done(Err(ShaderError::script("the shader process ended")));
    }
    match worker.replies.recv_timeout(deadline) {
        Ok(Ok((reply, pixels))) => {
            *slot = Some(worker);
            if reply["ok"].as_bool() == Some(true) {
                Outcome::Done(Ok(pixels))
            } else {
                Outcome::Done(Err(ShaderError::from_wire(&reply["error"])))
            }
        }
        Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
            worker.stop();
            Outcome::Done(Err(ShaderError::script(
                "the shader process ended during the run (out of memory?)",
            )))
        }
        Err(RecvTimeoutError::Timeout) => {
            worker.stop();
            Outcome::TimedOut
        }
    }
}

fn write_frame(out: &mut impl Write, header: &Value, payload: &[u8]) -> io::Result<()> {
    let header = serde_json::to_vec(header).map_err(io::Error::other)?;
    let len = u32::try_from(header.len()).map_err(io::Error::other)?;
    out.write_all(&len.to_le_bytes())?;
    out.write_all(&header)?;
    out.write_all(&(payload.len() as u64).to_le_bytes())?;
    out.write_all(payload)?;
    out.flush()
}

fn read_frame(input: &mut impl Read) -> io::Result<(Value, Vec<u8>)> {
    let mut len = [0; 4];
    input.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len);
    if len > MAX_HEADER {
        return Err(io::Error::other("shader frame header too large"));
    }
    let mut header = vec![0; len as usize];
    input.read_exact(&mut header)?;
    let mut size = [0; 8];
    input.read_exact(&mut size)?;
    let size = u64::from_le_bytes(size);
    if size > MAX_PAYLOAD {
        return Err(io::Error::other("shader frame payload too large"));
    }
    let mut payload = vec![0; size as usize];
    input.read_exact(&mut payload)?;
    let header = serde_json::from_slice(&header).map_err(io::Error::other)?;
    Ok((header, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_that_is_not_a_worker_is_not_started_again() {
        // This test binary cannot serve: it rejects the worker flag and exits.
        set_worker_program(std::env::current_exe().unwrap());
        let size = (8, 8, 0.0);
        let first = run("", &json!({}), size, None, Duration::from_secs(10));
        assert!(matches!(first, Outcome::Done(Err(_))));
        assert!(worker_pid().is_none());
        let started = std::time::Instant::now();
        let Outcome::Done(Err(again)) = run("", &json!({}), size, None, Duration::from_secs(10))
        else {
            panic!("a second run got further");
        };
        assert!(again.to_string().contains("not started again"), "{again}");
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "no new process"
        );
    }

    #[test]
    fn frames_round_trip_and_oversized_ones_are_refused() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &json!({"a": 1}), &[1, 2, 3]).unwrap();
        let (header, payload) = read_frame(&mut buf.as_slice()).unwrap();
        assert_eq!((header, payload), (json!({"a": 1}), vec![1, 2, 3]));
        let mut huge = Vec::new();
        huge.extend_from_slice(&(MAX_HEADER + 1).to_le_bytes());
        assert!(read_frame(&mut huge.as_slice()).is_err());
    }
}
