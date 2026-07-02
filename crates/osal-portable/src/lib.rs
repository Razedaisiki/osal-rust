//! Portable helper implementations.
//!
//! Provides reusable utilities that backends may optionally use:
//!
//! - Time conversion between ticks, `Duration`, `Milliseconds`,
//!   `Microseconds`
//! - Ring buffer for byte-oriented FIFO queues
//! - Static memory pools for `no_std` environments
//! - Fallback no-op implementations for optional features
//! - Wait/timeout helper functions
//!
//! These are **not** part of the public API surface. They are
//! internal building blocks that backends share.

#![no_std]

extern crate alloc;

pub mod byte_queue;
