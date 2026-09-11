//! FreeRTOS backend for the OSAL framework.
//!
//! Implements OSAL traits over a running FreeRTOS kernel.
//! The scheduler is owned by the application / BSP; this backend
//! is a guest of the kernel (ADR 0020).
//!
//! Capability status follows the terminology in
//! `docs/documentation-policy.md` (Validated / Implemented / Deferred):
//!
//! **Validated** (host + real-kernel on QEMU mps2-an385, Cortex-M3,
//! FreeRTOS Kernel V11.3.0 — isolated profiles in `integration/freertos-qemu-mps2/`):
//! - Mutex — P7G Step 4A (8 cases)
//! - CountingSemaphore / BinarySemaphore — P7G Step 4B (aggregate: 18 semaphore cases)
//! - Queue Core — P7G Step 4C-1 (9 cases)
//! - Queue Blocking — P7G Step 4C-2 + Step 4C-3 timeout/wake boundary-race closure (queue-blocking profile: 15 required cases = 1 harness + 14 Queue-specific cases)
//! - Task — P7G Step 4D (20 cases)
//! - Timer — P7G Step 4E (20 cases)
//! - System — Validated per README capability matrix (`xPortGetFreeHeapSize` + `taskENTER_CRITICAL`/`taskEXIT_CRITICAL`)
//! - Mixed-object integration / resource pressure — P7G Step 4F (`suite-mixed`: 6 required cases = 1 harness + 5 mixed-object cases)
//!
//! **Implemented** (host-contract-verified; QEMU exercised as part of
//! managed-object profiles, full promotion per README matrix):
//! - Clock — monotonic tick snapshots, chunked delay with per-chunk guard
//! - Runtime Lifecycle — init/shutdown/acquire (backend-local `RuntimeLifecycle`)
//!
//! **Deferred to P7G+:** ISR extensions.
//!
//! ## Validation layers
//!
//! - Host fixtures provide deterministic contract coverage (Virtual-mode
//!   fixture bridge with request/ack flush for Timer, sync fixtures for
//!   Mutex/Semaphore/Queue/Task).
//! - Real-kernel validation already exists on FreeRTOS V11.3.0 / QEMU
//!   mps2-an385 / Cortex-M3 via isolated profiles: `aggregate`,
//!   `queue-blocking`, `task`, `timer`, `mixed`.
//! - Physical MCU validation remains outstanding (deployment validation,
//!   not a P7G final-seal gate for the QEMU matrix).

#![no_std]

extern crate alloc;

pub mod clock;
pub mod mutex;
pub mod queue;
pub mod runtime;
pub mod semaphore;
pub mod system;
pub mod task;
pub mod timer;

#[cfg(feature = "integration-test-hooks")]
pub mod queue_hooks;
pub(crate) mod timer_control;
pub(crate) mod timer_service;
pub(crate) mod wait;
