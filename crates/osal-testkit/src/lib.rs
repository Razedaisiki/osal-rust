//! Testing utilities for OSAL backends.
//!
//! Provides shared infrastructure for testing OSAL implementations:
//!
//! - **Contract test harness**: Run the same behavior tests against
//!   every backend
//! - **Assertion helpers**: Common patterns for verifying OSAL
//!   behavior
//! - **Fake clock**: Deterministic time for reproducible tests
//! - **Fault injection**: Test error handling paths

#![no_std]

extern crate alloc;

// Modules to be populated in later phases:
// pub mod contract;
// pub mod assertions;
// pub mod fake_clock;
// pub mod fault;
