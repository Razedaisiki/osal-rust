//! Host runner for the shared queue demo.

mod support;

fn main() -> std::process::ExitCode {
    support::run("queue", osal_demo::queue::run)
}
