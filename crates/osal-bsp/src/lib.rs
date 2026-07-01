//! Board support package abstraction.
//!
//! Defines the interface for platform-specific services that sit
//! below the OSAL layer:
//!
//! - Boot and startup hooks
//! - Console / debug output
//! - Heap and memory region configuration
//! - Clock and timer hardware abstraction
//! - Interrupt controller and critical section config
//! - Resource limits (max tasks, max queues, etc.)
//!
//! BSP logic is intentionally separated from OS backend logic so
//! that platform details do not leak into the OSAL API.

#![no_std]

// Modules to be populated in later phases:
// pub mod boot;
// pub mod console;
// pub mod memory;
// pub mod clock;
// pub mod interrupt;
// pub mod resource;
// pub mod board;
