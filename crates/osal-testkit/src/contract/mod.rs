//! Generic contract tests for OSAL primitives.
//!
//! Each sub-module contains pure generic test functions that are
//! parameterized by a [`BackendFactory`](crate::factory::BackendFactory).
//! Backend crates call these functions from their own `#[test]`
//! entry points.
//!
//! # Usage
//!
//! ```ignore
//! use osal_testkit::contract::mutex;
//! use osal_testkit::factory::BackendFactory;
//!
//! #[test]
//! fn test_mutex_lock_unlock() {
//!     mutex::lock_unlock::<MyFactory>();
//! }
//! ```

pub mod clock;
pub mod lifetime;
pub mod mutex;
pub mod queue;
pub mod semaphore;
pub mod task;
