//! A shader's code is capped: CPU (Boa's instruction budget), `ArrayBuffer` size and GPU
//! memory. Skipped without a GPU adapter.

use std::time::{Duration, Instant};

use figstudio_shader::render;
use serde_json::json;

/// Runs `body` as the shader's `render`; `None` without a GPU.
fn run(body: &str) -> Option<Result<(), String>> {
    let main = format!("export function render(device: any, frame: any) {{ {body} }}");
    match render(&main, &json!({}), 8, 8, 0.0) {
        Ok(_) => Some(Ok(())),
        Err(e) if e.is_no_gpu() => None,
        Err(e) => {
            assert!(e.is_script(), "{e}");
            Some(Err(e.to_string()))
        }
    }
}

#[test]
fn endless_loops_stop_on_the_loop_limit() {
    worker();
    let Some(out) = run("for (;;) { for (let i = 0; i < 1000000; i++) {} }") else {
        return;
    };
    let error = out.unwrap_err();
    assert!(error.contains("iteration loops"), "{error}");
}

#[test]
fn endless_callbacks_stop_on_the_instruction_budget() {
    worker();
    // No JavaScript loop: a built-in calls the callback, a million times per level of a
    // recursion under its limit, so only the budget stops it.
    let Some(out) = run("const a = new Array(1000000).fill(0); \
         function f(): void { a.forEach(() => {}); f(); } f();")
    else {
        return;
    };
    // Boa's error, not the deadline's (which would say it ran past the time limit).
    let error = out.unwrap_err();
    assert!(error.contains("NoInstructionsRemain"), "{error}");
}

#[test]
fn a_built_in_that_never_returns_is_killed_at_the_deadline_and_not_run_again() {
    worker();
    if run("let s = 1;").is_none() {
        return;
    }
    let before = figstudio_shader::worker_pid();
    assert!(before.is_some(), "a run starts the shader process");
    let started = Instant::now();
    // No JavaScript runs while the built-in walks a trillion empty indexes: nothing inside the
    // process can stop it, so its process is killed at the 10 s deadline.
    let Some(out) = run("Array.prototype.forEach.call({ length: 1e12 }, () => {});") else {
        return;
    };
    let error = out.unwrap_err();
    assert!(error.contains("ran past"), "{error}");
    // The deadline, plus waiting for the other tests' runs (they share the one process).
    assert!(
        started.elapsed() < Duration::from_secs(40),
        "{:?}",
        started.elapsed()
    );
    // The same code is not run again; other code runs in a new process.
    let again = Instant::now();
    let error = run("Array.prototype.forEach.call({ length: 1e12 }, () => {});")
        .unwrap()
        .unwrap_err();
    assert!(error.contains("not run again"), "{error}");
    assert!(again.elapsed() < Duration::from_secs(1));
    // Its process was killed (not left spinning): the next run gets a new one.
    run("let s = 1;").unwrap().unwrap();
    assert_ne!(figstudio_shader::worker_pid(), before);
    #[cfg(unix)]
    if let Some(pid) = before {
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|s| s.success());
        assert!(!alive, "process {pid} still runs");
    }
}

#[test]
fn a_huge_array_buffer_is_refused() {
    worker();
    let Some(out) = run("new Uint8Array(300 * 1024 * 1024);") else {
        return;
    };
    let error = out.unwrap_err();
    assert!(error.contains("maximum buffer size"), "{error}");
}

#[test]
fn gpu_memory_past_the_cap_is_refused() {
    worker();
    let Some(out) = run("device.createBuffer({ size: 600 * 1024 * 1024, usage: 8 });") else {
        return;
    };
    let error = out.unwrap_err();
    assert!(error.contains("512 MiB"), "{error}");
}

#[test]
fn a_normal_script_still_runs() {
    worker();
    if let Some(out) = run("let s = 0; for (let i = 0; i < 100000; i++) { s += i; }") {
        out.unwrap();
    }
}

/// The shader process for these tests: this crate's worker binary (a test binary cannot be one).
fn worker() {
    figstudio_shader::set_worker_program(env!("CARGO_BIN_EXE_figstudio-shader-worker").into());
}
