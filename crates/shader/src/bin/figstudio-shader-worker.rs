//! A shader worker for this crate's tests, whose own binaries cannot be one (they point
//! `set_worker_program` at this). figstudio's programs serve as their own workers.

fn main() -> std::process::ExitCode {
    if figstudio_shader::serve_if_worker() {
        std::process::ExitCode::SUCCESS
    } else {
        eprintln!(
            "figstudio-shader-worker: started by figstudio with {}",
            figstudio_shader::WORKER_ARG
        );
        std::process::ExitCode::from(2)
    }
}
