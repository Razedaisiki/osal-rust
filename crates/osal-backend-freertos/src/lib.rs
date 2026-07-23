//! FreeRTOS backend for the OSAL framework.
//!
//! Implements OSAL traits over a running FreeRTOS kernel.
//! The scheduler is owned by the application / BSP; this backend
//! is a guest of the kernel (ADR 0020).
//!
//! # Current status (P7A)
//!
//! Only the runtime lifecycle (`initialize`, `shutdown`,
//! `runtime_state`) is implemented.  All OSAL primitive traits
//! (Queue, Mutex, Task, Timer, etc.) are deferred to P7B+.

#![no_std]

extern crate alloc;

pub mod runtime;
