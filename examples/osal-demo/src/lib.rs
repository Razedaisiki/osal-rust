//! Portable OSAL demo applications shared by POSIX and FreeRTOS.
//!
//! Each demo is a single application implementation written against the
//! `osal` facade only. Platform shells (host `main()`, FreeRTOS boot task)
//! are responsible for stdout/UART, scheduler bootstrap, and process or
//! firmware termination — never for demo logic.
//!
//! Demos return a typed report instead of printing; the platform shell
//! formats it. Because the report's `Display` implementation also lives
//! here, POSIX and FreeRTOS render identical bodies.

#![no_std]

extern crate alloc;

pub mod error;
pub mod mutex;
pub mod queue;
pub mod semaphore;
pub mod system;
pub mod task;
pub mod timer;

/// Multi-task pipeline demo.
///
/// Gated on the `pipeline` capability feature (enabled by the POSIX and
/// FreeRTOS backends, not by Mock) because it requires `Send` OSAL objects.
#[cfg(feature = "pipeline")]
pub mod pipeline_demo;

pub(crate) mod runtime;

pub use error::{DemoError, DemoResult};

// Exactly one OSAL backend must be selected for the demo build.
#[cfg(not(any(
    feature = "backend-posix",
    feature = "backend-mock",
    feature = "backend-freertos",
)))]
compile_error!(
    "exactly one OSAL backend must be selected for osal-demo \
     (backend-posix, backend-mock, or backend-freertos)"
);

#[cfg(any(
    all(feature = "backend-posix", feature = "backend-mock"),
    all(feature = "backend-posix", feature = "backend-freertos"),
    all(feature = "backend-mock", feature = "backend-freertos"),
))]
compile_error!("only one OSAL backend may be selected for osal-demo");
