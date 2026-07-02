//! OS-independent shared implementation layer.
//!
//! Provides common logic shared by all backends:
//!
//! - Object ID allocation and tracking
//! - Resource registration and lookup
//! - Parameter validation helpers
//! - Initialization lifecycle management
//!
//! This crate prevents each backend from inventing its own
//! object lifecycle and validation logic.

#![no_std]

pub mod close_state;
pub mod validation;
