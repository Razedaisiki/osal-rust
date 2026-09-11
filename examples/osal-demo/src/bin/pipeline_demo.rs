//! Host runner for the shared pipeline demo.

mod support;

use osal_demo::pipeline_demo::{PipelineEvent, PipelineReporter};

/// Renders the shared pipeline trace to stdout.
///
/// The event text comes from `PipelineEvent`'s `Display` in the shared
/// crate, so this prints the same body the FreeRTOS UART shell prints.
struct PosixPipelineReporter;

impl PipelineReporter for PosixPipelineReporter {
    fn report(&self, event: PipelineEvent) {
        println!("{event}");
    }
}

fn main() -> std::process::ExitCode {
    support::run("pipeline_demo", || {
        osal_demo::pipeline_demo::run_with_reporter(&PosixPipelineReporter)
    })
}
