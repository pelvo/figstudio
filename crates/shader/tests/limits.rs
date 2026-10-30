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
    let Some(out) = run("for (;;) { for (let i = 0; i < 1000000; i++) {} }") else {
        return;
    };
    let error = out.unwrap_err();
    assert!(error.contains("iteration loops"), "{error}");
}

#[test]
fn endless_callbacks_stop_on_the_instruction_budget() {
    let started = Instant::now();
    // No JavaScript loop: a built-in calls the callback, a million times per level of a
    // recursion under its limit, so only the budget stops it.
    let Some(out) = run("const a = new Array(1000000).fill(0); \
         function f(): void { a.forEach(() => {}); f(); } f();")
    else {
        return;
    };
    let error = out.unwrap_err();
    assert!(
        !error.contains("time limit") && !error.contains("ran past"),
        "{error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "stopped by the budget, before the 10 s deadline: {:?}",
        started.elapsed()
    );
}

#[test]
fn a_huge_array_buffer_is_refused() {
    let Some(out) = run("new Uint8Array(300 * 1024 * 1024);") else {
        return;
    };
    let error = out.unwrap_err();
    assert!(error.contains("maximum buffer size"), "{error}");
}

#[test]
fn gpu_memory_past_the_cap_is_refused() {
    let Some(out) = run("device.createBuffer({ size: 600 * 1024 * 1024, usage: 8 });") else {
        return;
    };
    let error = out.unwrap_err();
    assert!(error.contains("512 MiB"), "{error}");
}

#[test]
fn a_normal_script_still_runs() {
    if let Some(out) = run("let s = 0; for (let i = 0; i < 100000; i++) { s += i; }") {
        out.unwrap();
    }
}
