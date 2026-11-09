//! A shader worker ends itself when a job runs past its deadline plus a margin, so a stuck
//! run does not outlive a parent that quit or crashed during it.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// One frame of the worker's pipe protocol: a JSON header and raw bytes.
fn frame(header: &serde_json::Value) -> Vec<u8> {
    let header = serde_json::to_vec(header).unwrap();
    let mut out = (header.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&header);
    out.extend_from_slice(&0u64.to_le_bytes());
    out
}

#[test]
fn a_worker_whose_job_never_returns_ends_itself_without_its_parent() {
    // Without a GPU the worker answers at once and never runs the script.
    figstudio_shader::set_worker_program(env!("CARGO_BIN_EXE_figstudio-shader-worker").into());
    let probe = "export function render(device: any, frame: any) {}";
    if figstudio_shader::render(probe, &serde_json::json!({}), 8, 8, 0.0)
        .is_err_and(|e| e.skips_test())
    {
        return;
    }
    let mut worker = Command::new(env!("CARGO_BIN_EXE_figstudio-shader-worker"))
        .arg(figstudio_shader::WORKER_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = worker.stdout.take().unwrap();
    // It says it is ready first.
    let mut len = [0; 4];
    stdout.read_exact(&mut len).unwrap();
    // A built-in that never returns, with a 200 ms deadline; nobody kills the worker.
    let job = serde_json::json!({
        "js": "export function render() { Array.prototype.forEach.call({ length: 1e12 }, () => {}); }",
        "params": {}, "width": 8, "height": 8, "time_ms": 0, "deadline_ms": 200,
    });
    let mut stdin = worker.stdin.take().unwrap();
    stdin.write_all(&frame(&job)).unwrap();
    stdin.flush().unwrap();
    let started = Instant::now();
    loop {
        if let Some(status) = worker.try_wait().unwrap() {
            // Ended by its watchdog (the deadline plus the 5 s margin), not by a reply.
            assert_eq!(status.code(), Some(3), "{status}");
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the stuck worker kept running"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(stdin);
}

#[test]
fn a_worker_past_its_memory_cap_ends_itself() {
    figstudio_shader::set_worker_program(env!("CARGO_BIN_EXE_figstudio-shader-worker").into());
    let probe = "export function render(device: any, frame: any) {}";
    if figstudio_shader::render(probe, &serde_json::json!({}), 8, 8, 0.0)
        .is_err_and(|e| e.skips_test())
    {
        return;
    }
    // A cap of 256 MiB, and a string doubled past it (no array buffer, so only the cap
    // stops it), well inside the instruction budget.
    let mut worker = Command::new(env!("CARGO_BIN_EXE_figstudio-shader-worker"))
        .arg(figstudio_shader::WORKER_ARG)
        .env("FIGSTUDIO_SHADER_MAX_MEMORY", (256u64 << 20).to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = worker.stdout.take().unwrap();
    let mut len = [0; 4];
    stdout.read_exact(&mut len).unwrap();
    let job = serde_json::json!({
        "js": "export function render() { let s = 'x'; for (let i = 0; i < 31; i++) { s += s; } }",
        "params": {}, "width": 8, "height": 8, "time_ms": 0, "deadline_ms": 60_000,
    });
    let mut stdin = worker.stdin.take().unwrap();
    stdin.write_all(&frame(&job)).unwrap();
    stdin.flush().unwrap();
    let started = Instant::now();
    loop {
        if let Some(status) = worker.try_wait().unwrap() {
            assert_eq!(status.code(), Some(4), "{status}");
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the worker grew past its cap and kept running"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(stdin);
}
