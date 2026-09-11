//! FreeRTOS portable-demo runner — platform shell only.
//!
//! Contains **no** demo logic. It selects a demo at compile time and calls
//! the same `osal_demo::<name>::run()` function the POSIX host binaries
//! call, then renders the shared report through the MPS2 UART.

use core::ffi::c_char;
use core::fmt::{self, Write};

use osal_demo::DemoResult;
use osal_demo::pipeline_demo::{PipelineEvent, PipelineReporter};

unsafe extern "C" {
    fn console_write_byte(value: c_char);
}

/// Minimal UART writer bridge — no printf, no malloc, no newlib.
struct Console;

impl fmt::Write for Console {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            if byte == b'\n' {
                unsafe { console_write_byte(b'\r' as c_char) };
            }
            unsafe { console_write_byte(byte as c_char) };
        }
        Ok(())
    }
}

/// Renders the shared pipeline trace to the MPS2 UART.
///
/// The event text comes from `PipelineEvent`'s `Display` in the shared
/// crate, so the body matches the POSIX runner byte for byte.
struct FreertosReporter;

impl PipelineReporter for FreertosReporter {
    fn report(&self, event: PipelineEvent) {
        let mut out = Console;
        let _ = writeln!(out, "{event}");
    }
}

/// Demo selector resolved by `build.rs` from `OSAL_FREERTOS_DEMO`.
const DEMO: &str = env!("OSAL_FREERTOS_DEMO");

/// Entry point called from the firmware boot task.
///
/// Returns 0 on pass, 1 on failure — the C side maps non-zero to a
/// semihosting failure exit.
pub fn run() -> i32 {
    let mut out = Console;

    match DEMO {
        "mutex" => run_demo(&mut out, "mutex", osal_demo::mutex::run),
        "queue" => run_demo(&mut out, "queue", osal_demo::queue::run),
        "semaphore" => run_demo(&mut out, "semaphore", osal_demo::semaphore::run),
        "system" => run_demo(&mut out, "system", osal_demo::system::run),
        "task" => run_demo(&mut out, "task", osal_demo::task::run),
        "timer" => run_demo(&mut out, "timer", osal_demo::timer::run),
        "pipeline_demo" => run_demo(&mut out, "pipeline_demo", || {
            osal_demo::pipeline_demo::run_with_reporter(&FreertosReporter)
        }),
        _ => {
            // build.rs rejects unknown selectors at compile time; this is
            // unreachable defence only.
            let _ = writeln!(out, "OSAL_DEMO_FAIL name=unknown error=invalid-selector");
            let _ = writeln!(out, "OSAL_DEMO_END status=fail");
            1
        }
    }
}

fn run_demo<R>(
    out: &mut Console,
    name: &'static str,
    demo: impl FnOnce() -> DemoResult<R>,
) -> i32
where
    R: fmt::Display,
{
    let _ = writeln!(out, "OSAL_DEMO_BEGIN name={name} backend=freertos");

    match demo() {
        Ok(report) => {
            let _ = writeln!(out, "{report}");
            let _ = writeln!(out, "OSAL_DEMO_PASS name={name}");
            let _ = writeln!(out, "OSAL_DEMO_END status=pass");
            0
        }
        Err(error) => {
            let _ = writeln!(out, "OSAL_DEMO_FAIL name={name} error={error}");
            let _ = writeln!(out, "OSAL_DEMO_END status=fail");
            1
        }
    }
}
