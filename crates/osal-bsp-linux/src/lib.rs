//! Linux host board support package.
//!
//! Provides platform-specific implementations for development on
//! Linux hosts:
//!
//! - Console output via stdout/stderr
//! - Monotonic clock via `CLOCK_MONOTONIC`
//! - Memory configuration for host processes
//! - Boot/startup hooks for development and CI

#![no_std]

// Modules to be populated in later phases:
// pub mod console;
// pub mod clock;
// pub mod memory;
// pub mod boot;
