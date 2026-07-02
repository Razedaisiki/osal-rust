//! Testing infrastructure for OSAL backends.
//!
//! Provides:
//!
//! - **[`factory`]** — capability-based factory traits for creating
//!   primitives in a backend-agnostic way.
//! - **[`contract`]** — generic contract test functions that run
//!   against any backend implementing the required factories.
//! - **[`assertions`]** — no-std-compatible assertion macros.

#![no_std]

extern crate alloc;

pub mod assertions;
pub mod contract;
pub mod factory;
pub mod fixture;
