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

// Modules to be populated in later phases:
// pub mod time_convert;
// pub mod ring_buffer;
// pub mod static_pool;
// pub mod fallback;
