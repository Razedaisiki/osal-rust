//! Object lifetime contract tests.
//!
//! These tests verify the ownership and lifetime rules defined in
//! `docs/object-lifetime.md`.
//!
//! Tests in this module require `Clone` support on the tested object.
//! Backends whose types do not implement `Clone` skip these tests.

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue as _;

use crate::factory::QueueFactory;

// ---------------------------------------------------------------------------
// Clone-based lifetime tests
// ---------------------------------------------------------------------------

/// Two cloned handles refer to the same backend object.
///
/// Sending through one handle makes the message visible to the other.
pub fn clone_handle_shares_state<F: QueueFactory>(factory: &F)
where
    F::Queue: Clone,
{
    let q1 = factory.create_queue(4, 2).unwrap();
    let q2 = q1.clone();

    q1.send(&[1, 2], Timeout::NoWait).unwrap();

    let mut out = [0u8; 2];
    q2.recv(&mut out, Timeout::NoWait).unwrap();

    assert_eq!(out, [1, 2]);
}

/// Dropping one clone does not destroy the shared backend resource.
pub fn drop_clone_keeps_object_alive<F: QueueFactory>(factory: &F)
where
    F::Queue: Clone,
{
    let q1 = factory.create_queue(4, 2).unwrap();
    let q2 = q1.clone();

    q1.send(&[1, 2], Timeout::NoWait).unwrap();
    drop(q1);

    let mut out = [0u8; 2];
    q2.recv(&mut out, Timeout::NoWait).unwrap();
    assert_eq!(out, [1, 2]);
}

/// Closing through one handle affects all clones.
pub fn close_affects_all_clones<F: QueueFactory>(factory: &F)
where
    F::Queue: Clone,
{
    let q1 = factory.create_queue(4, 2).unwrap();
    let q2 = q1.clone();

    q1.close().unwrap();

    // Both handles see the queue as closed.
    let result = q2.send(&[1, 2], Timeout::NoWait);
    assert!(matches!(result, Err(Error::QueueClosed)));
}

// ---------------------------------------------------------------------------
// Grouped entry point
// ---------------------------------------------------------------------------

/// All clone-based lifetime contract tests.
pub fn run_clone_contracts<F: QueueFactory>(factory: &F)
where
    F::Queue: Clone,
{
    clone_handle_shares_state::<F>(factory);
    drop_clone_keeps_object_alive::<F>(factory);
    close_affects_all_clones::<F>(factory);
}
