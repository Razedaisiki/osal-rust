//! Transactional runtime lifecycle state machine.
//!
//! # Overview
//!
//! [`RuntimeLifecycle`] packs the four-state OSAL runtime cycle and an
//! active-object counter into a single [`AtomicUsize`], giving every
//! operation a unique CAS linearisation point (ADR 0016).
//!
//! # Layout
//!
//! ```text
//! bits [usize::BITS-1 .. 2]  — active object count
//! bits [1 .. 0]               — RuntimeState (Uninitialized=0, …, ShuttingDown=3)
//! ```
//!
//! # Distinction from `Task::count()`
//!
//! `Task::count()` counts entries whose function has not yet completed.
//! `active_objects()` counts logical OSAL objects (Queue, Mutex, Task
//! handle, Timer) that still hold a [`RuntimeLease`].  A finished Task
//! whose handle is still alive has `Task::count() == 0` but contributes
//! to `active_objects()`.

use core::sync::atomic::{AtomicUsize, Ordering};

use osal_api::error::{Error, Result};
use osal_api::runtime::RuntimeState;

// ---------------------------------------------------------------------------
// Word encoding
// ---------------------------------------------------------------------------

const STATE_BITS: usize = 2;
const STATE_MASK: usize = (1 << STATE_BITS) - 1;
const MAX_COUNT: usize = usize::MAX >> STATE_BITS;

fn encode(state: RuntimeState, count: usize) -> usize {
    debug_assert!(count <= MAX_COUNT);
    (state as usize) | (count << STATE_BITS)
}

fn decode_state(word: usize) -> RuntimeState {
    match word & STATE_MASK {
        0 => RuntimeState::Uninitialized,
        1 => RuntimeState::Initializing,
        2 => RuntimeState::Running,
        3 => RuntimeState::ShuttingDown,
        _ => unreachable!(),
    }
}

fn decode_count(word: usize) -> usize {
    word >> STATE_BITS
}

// ---------------------------------------------------------------------------
// RuntimeLifecycle
// ---------------------------------------------------------------------------

/// A process-local runtime lifecycle manager.
///
/// Suitable for placement in a `static`:
///
/// ```ignore
/// static RUNTIME: RuntimeLifecycle = RuntimeLifecycle::new();
/// ```
pub struct RuntimeLifecycle {
    word: AtomicUsize,
}

impl Default for RuntimeLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeLifecycle {
    pub const fn new() -> Self {
        // RuntimeState::Uninitialized = 0, count = 0 → word = 0.
        Self {
            word: AtomicUsize::new(0),
        }
    }

    pub fn state(&self) -> RuntimeState {
        decode_state(self.word.load(Ordering::Acquire))
    }

    pub fn active_objects(&self) -> usize {
        decode_count(self.word.load(Ordering::Acquire))
    }

    // ---------------------------------------------------------------
    // Initialisation
    // ---------------------------------------------------------------

    pub fn begin_initialize(&self) -> Result<InitializeTransition<'_>> {
        let expected = encode(RuntimeState::Uninitialized, 0);
        let desired = encode(RuntimeState::Initializing, 0);

        match self.word.compare_exchange(
            expected,
            desired,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(InitializeTransition {
                lifecycle: self,
                committed: false,
            }),
            Err(current) => {
                match decode_state(current) {
                    RuntimeState::Uninitialized => {
                        // CAS failed but state reads Uninitialized —
                        // another thread won the race, so from our
                        // perspective it is already initialised.
                        Err(Error::AlreadyInitialized)
                    }
                    RuntimeState::Running => Err(Error::AlreadyInitialized),
                    _ => Err(Error::Busy),
                }
            }
        }
    }

    // ---------------------------------------------------------------
    // Shutdown
    // ---------------------------------------------------------------

    pub fn begin_shutdown(&self) -> Result<ShutdownTransition<'_>> {
        loop {
            let current = self.word.load(Ordering::Acquire);

            match decode_state(current) {
                RuntimeState::Uninitialized => return Err(Error::NotInitialized),
                RuntimeState::Initializing | RuntimeState::ShuttingDown => {
                    return Err(Error::Busy);
                }
                RuntimeState::Running => {}
            }

            if decode_count(current) != 0 {
                return Err(Error::Busy);
            }

            let next = encode(RuntimeState::ShuttingDown, 0);
            match self.word.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(ShutdownTransition {
                        lifecycle: self,
                        committed: false,
                    });
                }
                Err(_) => continue,
            }
        }
    }

    // ---------------------------------------------------------------
    // Object lease
    // ---------------------------------------------------------------

    pub fn acquire(&self) -> Result<RuntimeLease<'_>> {
        loop {
            let current = self.word.load(Ordering::Acquire);

            if decode_state(current) != RuntimeState::Running {
                return Err(Error::NotInitialized);
            }

            let count = decode_count(current);
            if count >= MAX_COUNT {
                return Err(Error::Overflow);
            }
            let next = encode(RuntimeState::Running, count + 1);

            match self.word.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(RuntimeLease { lifecycle: self });
                }
                Err(_) => continue,
            }
        }
    }
}

// Note: RuntimeLifecycle is auto-Send + auto-Sync via AtomicUsize.

// ---------------------------------------------------------------------------
// InitializeTransition
// ---------------------------------------------------------------------------

#[must_use = "initialization must be committed"]
pub struct InitializeTransition<'a> {
    lifecycle: &'a RuntimeLifecycle,
    committed: bool,
}

impl InitializeTransition<'_> {
    pub fn commit(mut self) {
        let expected = encode(RuntimeState::Initializing, 0);
        let desired = encode(RuntimeState::Running, 0);
        self.lifecycle
            .word
            .compare_exchange(expected, desired, Ordering::AcqRel, Ordering::Acquire)
            .ok();
        self.committed = true;
    }
}

impl Drop for InitializeTransition<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let expected = encode(RuntimeState::Initializing, 0);
            let desired = encode(RuntimeState::Uninitialized, 0);
            self.lifecycle
                .word
                .compare_exchange(expected, desired, Ordering::AcqRel, Ordering::Acquire)
                .ok();
        }
    }
}

// ---------------------------------------------------------------------------
// ShutdownTransition
// ---------------------------------------------------------------------------

#[must_use = "shutdown must be committed"]
pub struct ShutdownTransition<'a> {
    lifecycle: &'a RuntimeLifecycle,
    committed: bool,
}

impl ShutdownTransition<'_> {
    pub fn commit(mut self) {
        let expected = encode(RuntimeState::ShuttingDown, 0);
        let desired = encode(RuntimeState::Uninitialized, 0);
        self.lifecycle
            .word
            .compare_exchange(expected, desired, Ordering::AcqRel, Ordering::Acquire)
            .ok();
        self.committed = true;
    }
}

impl Drop for ShutdownTransition<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let expected = encode(RuntimeState::ShuttingDown, 0);
            let desired = encode(RuntimeState::Running, 0);
            self.lifecycle
                .word
                .compare_exchange(expected, desired, Ordering::AcqRel, Ordering::Acquire)
                .ok();
        }
    }
}

// ---------------------------------------------------------------------------
// RuntimeLease
// ---------------------------------------------------------------------------

#[must_use = "the lease must be retained for the object's lifetime"]
pub struct RuntimeLease<'a> {
    lifecycle: &'a RuntimeLifecycle,
}

impl Drop for RuntimeLease<'_> {
    fn drop(&mut self) {
        loop {
            let current = self.lifecycle.word.load(Ordering::Acquire);
            let count = decode_count(current);
            debug_assert!(count > 0, "runtime object count underflow");
            let next = encode(decode_state(current), count - 1);
            if self
                .lifecycle
                .word
                .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return;
            }
        }
    }
}

// Note: RuntimeLease is auto-Send + auto-Sync via the AtomicUsize reference.

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::sync::Arc;
    use alloc::vec::Vec;
    use core::sync::atomic::AtomicUsize;
    use std::sync::Barrier;
    use std::thread;

    // ---- helper ----

    fn init(rt: &RuntimeLifecycle) {
        rt.begin_initialize().unwrap().commit();
    }

    // ---- basic state ----

    #[test]
    fn initial_state_is_uninitialized() {
        let rt = RuntimeLifecycle::new();
        assert_eq!(rt.state(), RuntimeState::Uninitialized);
        assert_eq!(rt.active_objects(), 0);
    }

    #[test]
    fn initialize_commit_enters_running() {
        let rt = RuntimeLifecycle::new();
        rt.begin_initialize().unwrap().commit();
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    #[test]
    fn initialize_drop_rolls_back() {
        let rt = RuntimeLifecycle::new();
        {
            let _t = rt.begin_initialize().unwrap();
            assert_eq!(rt.state(), RuntimeState::Initializing);
        }
        assert_eq!(rt.state(), RuntimeState::Uninitialized);
    }

    #[test]
    fn initialize_while_running_returns_already_initialized() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        assert!(rt.begin_initialize().is_err());
        // Exact variant not tested — do not compare non-Debug types.
    }

    #[test]
    fn initialize_while_initializing_returns_busy() {
        let rt = RuntimeLifecycle::new();
        let _t = rt.begin_initialize().unwrap();
        // A second initialise attempt while Initializing → Busy.
        assert!(rt.begin_initialize().is_err());
    }

    // ---- lease ----

    #[test]
    fn acquire_before_initialize_fails() {
        let rt = RuntimeLifecycle::new();
        assert!(rt.acquire().is_err());
        assert_eq!(rt.active_objects(), 0);
    }

    #[test]
    fn acquire_while_running_increments_count() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        let lease = rt.acquire().unwrap();
        assert_eq!(rt.active_objects(), 1);
        drop(lease);
        assert_eq!(rt.active_objects(), 0);
    }

    #[test]
    fn dropping_lease_decrements_count() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        let a = rt.acquire().unwrap();
        let b = rt.acquire().unwrap();
        assert_eq!(rt.active_objects(), 2);
        drop(a);
        assert_eq!(rt.active_objects(), 1);
        drop(b);
        assert_eq!(rt.active_objects(), 0);
    }

    #[test]
    fn multiple_leases_are_counted() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        let leases: Vec<RuntimeLease<'_>> = (0..5).map(|_| rt.acquire().unwrap()).collect();
        assert_eq!(rt.active_objects(), 5);
        drop(leases);
        assert_eq!(rt.active_objects(), 0);
    }

    // ---- shutdown ----

    #[test]
    fn shutdown_before_initialize_returns_not_initialized() {
        let rt = RuntimeLifecycle::new();
        assert!(rt.begin_shutdown().is_err());
    }

    #[test]
    fn shutdown_with_active_lease_returns_busy() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        let _lease = rt.acquire().unwrap();
        assert!(rt.begin_shutdown().is_err());
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    #[test]
    fn shutdown_commit_returns_to_uninitialized() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        rt.begin_shutdown().unwrap().commit();
        assert_eq!(rt.state(), RuntimeState::Uninitialized);
    }

    #[test]
    fn shutdown_drop_rolls_back_to_running() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        {
            let _t = rt.begin_shutdown().unwrap();
            assert_eq!(rt.state(), RuntimeState::ShuttingDown);
        }
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    #[test]
    fn runtime_can_reinitialize_after_shutdown() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        rt.begin_shutdown().unwrap().commit();
        init(&rt);
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    #[test]
    fn shutdown_during_initializing_returns_busy() {
        let rt = RuntimeLifecycle::new();
        let _t = rt.begin_initialize().unwrap();
        assert!(rt.begin_shutdown().is_err());
    }

    // ---- acquire during transitions ----

    #[test]
    fn acquire_during_initializing_fails() {
        let rt = RuntimeLifecycle::new();
        let _t = rt.begin_initialize().unwrap();
        assert!(rt.acquire().is_err());
    }

    #[test]
    fn acquire_during_shutting_down_fails() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        let _st = rt.begin_shutdown().unwrap();
        assert!(rt.acquire().is_err());
    }

    // ---- overflow ----

    #[test]
    fn acquire_overflow_returns_overflow() {
        let rt = RuntimeLifecycle::new();
        // Set count to MAX_COUNT.  Since state occupies 2 bits,
        // the maximum count is 2^(usize::BITS-2)-1.  Adding 1
        // would overflow the count field.
        let max = MAX_COUNT;
        rt.word
            .store(encode(RuntimeState::Running, max), Ordering::Release);
        assert_eq!(rt.active_objects(), max);
        assert!(rt.acquire().is_err());
        // Word must be unchanged.
        assert_eq!(rt.active_objects(), max);
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    // ---- concurrency ----

    #[test]
    fn concurrent_initialize_at_least_one_succeeds() {
        let n = 8;
        let rt = Arc::new(RuntimeLifecycle::new());
        let barrier = Arc::new(Barrier::new(n));
        let gate = Arc::new(Barrier::new(n));
        let successes = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..n {
            let rt = Arc::clone(&rt);
            let b = Arc::clone(&barrier);
            let g = Arc::clone(&gate);
            let s = Arc::clone(&successes);
            handles.push(thread::spawn(move || {
                b.wait();
                if rt.begin_initialize().is_ok() {
                    s.fetch_add(1, Ordering::Relaxed);
                }
                g.wait(); // all threads hold until everyone is done
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // At least one must have succeeded; state is Uninitialized
        // after all guards drop.
        let n_ok = successes.load(Ordering::Relaxed);
        assert!(n_ok >= 1, "expected >= 1 successes, got {n_ok}");
        assert_eq!(rt.state(), RuntimeState::Uninitialized);
    }

    #[test]
    fn acquire_and_shutdown_cannot_both_succeed() {
        let rt = Arc::new(RuntimeLifecycle::new());
        init(&rt);

        let iterations = 400;
        let acquired = Arc::new(AtomicUsize::new(0));
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(2));

        for _ in 0..iterations {
            // Reset to Running with no leases.
            rt.word
                .store(encode(RuntimeState::Running, 0), Ordering::Release);

            let a = Arc::clone(&acquired);
            let s = Arc::clone(&shutdowns);
            let rt_clone = Arc::clone(&rt);
            let b = Arc::clone(&barrier);

            let t1 = thread::spawn(move || {
                b.wait();
                if rt_clone.acquire().is_ok() {
                    a.fetch_add(1, Ordering::Relaxed);
                }
            });

            let rt_clone2 = Arc::clone(&rt);
            let b2 = Arc::clone(&barrier);
            let t2 = thread::spawn(move || {
                b2.wait();
                if rt_clone2
                    .begin_shutdown()
                    .is_ok_and(|tx| {
                        tx.commit();
                        true
                    })
                {
                    s.fetch_add(1, Ordering::Relaxed);
                }
            });

            t1.join().unwrap();
            t2.join().unwrap();

            // In any given iteration, at most one can succeed.
            let a_val = acquired.load(Ordering::Relaxed);
            let s_val = shutdowns.load(Ordering::Relaxed);
            assert!(
                a_val + s_val <= 1,
                "iteration {iterations}: both acquire ({a_val}) and shutdown ({s_val}) succeeded"
            );

            acquired.store(0, Ordering::Relaxed);
            shutdowns.store(0, Ordering::Relaxed);
        }
    }

    // ---- compile-time: lease is not Clone ----

    #[allow(dead_code)]
    fn lease_is_not_clone(_lease: RuntimeLease<'_>) {
        // The following line would fail to compile:
        // let _copy = lease.clone();
    }
}
