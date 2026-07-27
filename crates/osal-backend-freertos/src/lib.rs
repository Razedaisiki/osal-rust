//! FreeRTOS backend for the OSAL framework.
//!
//! Implements OSAL traits over a running FreeRTOS kernel.
//! The scheduler is owned by the application / BSP; this backend
//! is a guest of the kernel (ADR 0020).
//!
//! # Current status (P7D)
//!
//! Capability status follows the terminology in
//! `docs/documentation-policy.md`:
//!
//! **Implemented** (host-contract-verified):
//! - Runtime lifecycle — init/shutdown/acquire lifecycle tested
//! - Clock — monotonic tick snapshots, chunked delay with per-chunk guard
//! - System — heap introspection, nesting critical sections
//! - Mutex — native priority-inheritance, RAII guard, !Send+!Sync
//! - CountingSemaphore — kernel count sole source of truth
//! - BinarySemaphore — native binary semaphore, initial unsignaled
//! - Queue — ByteQueue + native mutex + dual wake semaphore, waiter-credit
//!   protocol, close-drain broadcast
//!
//! **Validated** (host + FreeRTOS kernel integration tested):
//! - *(none yet — requires real FreeRTOS runtime tests)*
//!
//! **Deferred to P7E+:** Task, Timer, ISR extensions.
//!
//! ## Implementation vs Validation
//!
//! All primitives pass Linux-host fixture contract tests including
//! cross-thread blocking and wake-one semantics.  Promotion from
//! **Implemented** to **Validated** requires running these tests
//! against a real FreeRTOS kernel (QEMU or physical MCU) to verify
//! priority inheritance, real tick-interrupt timing, and kernel-level
//! waiter scheduling.

#![no_std]

extern crate alloc;

pub mod clock;
pub mod mutex;
pub mod queue;
pub mod runtime;
pub mod semaphore;
pub mod system;
pub(crate) mod wait;
