//! Thin FFI wrappers around POSIX primitives.
//!
//! All `unsafe` is contained in this module. Backend code uses
//! these safe wrappers instead of calling libc directly.

pub mod condvar;
pub mod errno;
pub mod mutex;
pub mod recursive_mutex;
pub mod task;
pub mod time;
