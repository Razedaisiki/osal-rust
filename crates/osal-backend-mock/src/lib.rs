//! Mock backend for testing and simulation.
//!
//! Provides deterministic, in-memory implementations of OSAL traits
//! for unit testing and contract verification.

#![no_std]

extern crate alloc;

pub mod clock;
pub mod fault;
pub mod queue;
pub mod wait;
