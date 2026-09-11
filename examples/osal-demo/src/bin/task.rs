//! Host runner for the shared task demo.

mod support;

fn main() -> std::process::ExitCode {
    support::run("task", osal_demo::task::run)
}
