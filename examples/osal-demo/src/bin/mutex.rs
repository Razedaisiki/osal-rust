//! Host runner for the shared mutex demo.

mod support;

fn main() -> std::process::ExitCode {
    support::run("mutex", osal_demo::mutex::run)
}
