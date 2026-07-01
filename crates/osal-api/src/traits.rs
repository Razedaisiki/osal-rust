//! Trait definitions for OSAL abstractions.
//!
//! Each sub-module defines the contract that backend implementations
//! must fulfill. The traits are designed to be implementable across
//! diverse platforms — POSIX hosts, real-time kernels, and mock
//! environments.
//!
//! Backend crates implement these traits for their target platform.
//! Users write application code against the traits via the `osal`
//! facade crate.

pub mod clock;
pub mod mutex;
pub mod queue;
pub mod semaphore;
pub mod system;
pub mod task;
pub mod timer;
