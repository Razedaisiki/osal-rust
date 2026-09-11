//! Host runner for the shared system demo.

mod support;

fn main() -> std::process::ExitCode {
    support::run("system", osal_demo::system::run)
}
