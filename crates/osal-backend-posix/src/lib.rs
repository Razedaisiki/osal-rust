//! POSIX backend implementation.
//!
//! Implements OSAL traits using POSIX primitives:
//! - pthread_mutex_t / pthread_cond_t for synchronization
//! - clock_gettime(CLOCK_MONOTONIC) for timing
//! - malloc/free via libc for allocation

#![no_std]

extern crate alloc;

pub(crate) mod sys;
