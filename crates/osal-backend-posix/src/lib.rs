//! POSIX backend implementation.
//!
//! Implements OSAL traits using POSIX primitives:
//! - pthread_mutex_t / pthread_cond_t for synchronization
//! - clock_gettime(CLOCK_MONOTONIC) for timing
//! - malloc/free via libc for allocation

#![no_std]

extern crate alloc;

#[cfg(any(test, feature = "testkit"))]
extern crate std;

pub mod clock;
pub mod mutex;
pub mod queue;
pub mod semaphore;
pub(crate) mod sys;
pub mod system;
pub mod task;
pub mod timer;
pub(crate) mod timer_service;
