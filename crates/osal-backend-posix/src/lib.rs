//! POSIX backend implementation.
//!
//! Implements the OSAL API using POSIX primitives:
//!
//! - **Tasks**: `pthread_create` / `pthread_join`
//! - **Mutex**: `pthread_mutex_t` (error-check + recursive)
//! - **Condition variables**: `pthread_cond_t` for blocking
//! - **Clock**: `clock_gettime(CLOCK_MONOTONIC)`
//! - **Heap**: `malloc` / `free` via libc
//!
//! This backend serves as the primary development, testing, and CI
//! platform. It runs on Linux, macOS, and other POSIX-compatible
//! systems.

#![no_std]

extern crate alloc;

// Modules to be populated in later phases:
// pub mod task;
// pub mod mutex;
// pub mod semaphore;
// pub mod queue;
// pub mod timer;
// pub mod clock;
// mod sys;
