//! Host runner for the shared semaphore demo.

mod support;

fn main() -> std::process::ExitCode {
    support::run("semaphore", osal_demo::semaphore::run)
}
