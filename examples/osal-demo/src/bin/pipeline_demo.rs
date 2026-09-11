//! Host runner for the shared pipeline demo.

mod support;

fn main() -> std::process::ExitCode {
    support::run("pipeline_demo", osal_demo::pipeline_demo::run)
}
