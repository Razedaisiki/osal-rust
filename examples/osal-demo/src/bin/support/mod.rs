//! Host runner shell — process bootstrap, stdout, and exit code only.
//!
//! This is deliberately the *only* host-specific code: it invokes the same
//! `osal_demo::<name>::run()` functions that the FreeRTOS firmware calls.

use osal_demo::DemoResult;

#[cfg(feature = "backend-posix")]
const BACKEND: &str = "posix";

#[cfg(feature = "backend-mock")]
const BACKEND: &str = "mock";

#[cfg(not(any(feature = "backend-posix", feature = "backend-mock")))]
const BACKEND: &str = "freertos";

/// Run one demo, emitting the shared protocol markers.
///
/// ```text
/// OSAL_DEMO_BEGIN name=<name> backend=<backend>
/// <report Display (shared crate)>
/// OSAL_DEMO_PASS name=<name>
/// OSAL_DEMO_END status=pass
/// ```
pub fn run<R>(name: &'static str, demo: impl FnOnce() -> DemoResult<R>) -> std::process::ExitCode
where
    R: core::fmt::Display,
{
    println!("OSAL_DEMO_BEGIN name={name} backend={BACKEND}");

    match demo() {
        Ok(report) => {
            println!("{report}");
            println!("OSAL_DEMO_PASS name={name}");
            println!("OSAL_DEMO_END status=pass");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            println!("OSAL_DEMO_FAIL name={name} error={error}");
            println!("OSAL_DEMO_END status=fail");
            std::process::ExitCode::FAILURE
        }
    }
}
