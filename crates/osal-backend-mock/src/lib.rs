//! Mock backend for testing and simulation.
//!
//! Provides deterministic, fake implementations of all OSAL traits
//! for unit testing and contract verification. The mock backend:
//!
//! - Simulates time deterministically (no real clock)
//! - Allows fault injection for error-path testing
//! - Records operation history for assertions
//! - Runs without any OS dependencies
//!
//! Used with `osal-testkit` to run contract tests without a real
//! operating system backend.

#![no_std]

extern crate alloc;

// Modules to be populated in later phases:
// pub mod fake_task;
// pub mod fake_mutex;
// pub mod fake_semaphore;
// pub mod fake_queue;
// pub mod fake_timer;
// pub mod fake_clock;
// pub mod fault;
