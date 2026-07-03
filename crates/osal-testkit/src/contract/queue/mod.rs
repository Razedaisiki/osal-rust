//! Contract tests for the [`Queue`] trait.
//!
//! Split into two groups:
//!
//! - [`QueueCoreContract`]: tests that all backends must pass (Mock + POSIX).
//! - [`QueueBlockingContract`]: tests requiring real concurrent blocking
//!   (POSIX only during P0; Mock deferred until scheduler is added).
//!
//! See `docs/behavior-contract.md#11-queue-contract` for the full spec.

mod creation;
mod fifo;
mod error_precedence;
mod close;
mod timeout;

use crate::factory::QueueFactory;

/// Core contract tests — all backends must pass.
pub fn run_core_contracts<F: QueueFactory>(factory: &F) {
    creation::run::<F>(factory);
    fifo::run::<F>(factory);
    error_precedence::run::<F>(factory);
    close::run::<F>(factory);
    timeout::run::<F>(factory);
}
