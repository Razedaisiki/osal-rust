//! # OSAL API
//!
//! This crate defines the public API traits and types for the OSAL framework.
//! It has no runtime dependencies and is `no_std` compatible.
//!
//! ## Architecture
//!
//! `osal-api` is the foundation crate. It declares **what** OSAL can do,
//! not **how** it does it. Backend crates (`osal-backend-posix`,
//! `osal-backend-freertos`, `osal-backend-mock`) implement these traits for
//! specific platforms.
//!
//! End users should use the `osal` facade crate instead of depending on
//! `osal-api` directly.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod error;
pub mod prelude;
pub mod time;
pub mod traits;
pub mod types;
