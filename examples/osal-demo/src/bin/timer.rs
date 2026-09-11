//! Host runner for the shared timer demo.

mod support;

fn main() -> std::process::ExitCode {
    support::run("timer", osal_demo::timer::run)
}
