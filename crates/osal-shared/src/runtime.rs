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
        // Disarm Drop BEFORE the assertion, so a panic won't
        // double-attempt rollback.
        self.committed = true;

        let result = self.lifecycle.word.compare_exchange(
            encode(RuntimeState::Initializing, 0),
            encode(RuntimeState::Running, 0),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        assert!(
            result.is_ok(),
            "runtime initialize commit invariant violated"
        );
    }
}

impl Drop for InitializeTransition<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let result = self.lifecycle.word.compare_exchange(
                encode(RuntimeState::Initializing, 0),
                encode(RuntimeState::Uninitialized, 0),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            debug_assert!(
                result.is_ok(),
                "runtime initialize rollback invariant violated"
            );
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
        self.committed = true;

        let result = self.lifecycle.word.compare_exchange(
            encode(RuntimeState::ShuttingDown, 0),
            encode(RuntimeState::Uninitialized, 0),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        assert!(
            result.is_ok(),
            "runtime shutdown commit invariant violated"
        );
    }
}

impl Drop for ShutdownTransition<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let result = self.lifecycle.word.compare_exchange(
                encode(RuntimeState::ShuttingDown, 0),
                encode(RuntimeState::Running, 0),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            debug_assert!(
                result.is_ok(),
                "runtime shutdown rollback invariant violated"
            );
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

            assert_eq!(
                decode_state(current),
                RuntimeState::Running,
                "runtime lease dropped outside Running state"
            );

            let count = decode_count(current);
            let next_count = count
                .checked_sub(1)
                .expect("runtime object count underflow");
            let next = encode(RuntimeState::Running, next_count);

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
        assert!(matches!(
            rt.begin_initialize(),
            Err(Error::AlreadyInitialized)
        ));
    }

    #[test]
    fn initialize_while_initializing_returns_busy() {
        let rt = RuntimeLifecycle::new();
        let _t = rt.begin_initialize().unwrap();
        assert!(matches!(rt.begin_initialize(), Err(Error::Busy)));
    }

    #[test]
    fn initialize_during_shutting_down_returns_busy() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        let _st = rt.begin_shutdown().unwrap();
        assert!(matches!(rt.begin_initialize(), Err(Error::Busy)));
    }

    // ---- lease ----

    #[test]
    fn acquire_before_initialize_fails() {
        let rt = RuntimeLifecycle::new();
        assert!(matches!(rt.acquire(), Err(Error::NotInitialized)));
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
        assert!(matches!(
            rt.begin_shutdown(),
            Err(Error::NotInitialized)
        ));
    }

    #[test]
    fn shutdown_with_active_lease_returns_busy() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        let _lease = rt.acquire().unwrap();
        assert!(matches!(rt.begin_shutdown(), Err(Error::Busy)));
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    #[test]
    fn shutdown_during_initializing_returns_busy() {
        let rt = RuntimeLifecycle::new();
        let _t = rt.begin_initialize().unwrap();
        assert!(matches!(rt.begin_shutdown(), Err(Error::Busy)));
    }

    #[test]
    fn shutdown_during_shutting_down_returns_busy() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        let _st = rt.begin_shutdown().unwrap();
        assert!(matches!(rt.begin_shutdown(), Err(Error::Busy)));
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

    // ---- acquire during transitions ----

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
        assert!(matches!(rt.acquire(), Err(Error::Overflow)));
        // Word must be unchanged.
        assert_eq!(rt.active_objects(), max);
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    // ---- acquire during transitions ----

    #[test]
    fn acquire_during_initializing_returns_not_initialized() {
        let rt = RuntimeLifecycle::new();
        let _t = rt.begin_initialize().unwrap();
        assert!(matches!(rt.acquire(), Err(Error::NotInitialized)));
    }

    #[test]
    fn acquire_during_shutting_down_returns_not_initialized() {
        let rt = RuntimeLifecycle::new();
        init(&rt);
        let _st = rt.begin_shutdown().unwrap();
        assert!(matches!(rt.acquire(), Err(Error::NotInitialized)));
    }

    // ---- concurrency ----

    #[test]
    #[ignore = "requires multiple hardware threads — run manually or in CI with sufficient cores"]
    fn only_one_concurrent_initialize_succeeds() {
        let n: usize = 4;
        let rt = Arc::new(RuntimeLifecycle::new());

        let start = Arc::new(Barrier::new(n));
        let attempted = Arc::new(Barrier::new(n + 1)); // +1 for main
        let release = Arc::new(Barrier::new(n + 1));
        let successes = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..n {
            let rt = Arc::clone(&rt);
            let start = Arc::clone(&start);
            let attempted = Arc::clone(&attempted);
            let release = Arc::clone(&release);
            let successes = Arc::clone(&successes);

            handles.push(thread::spawn(move || {
                start.wait();

                // Hold the transition — must not drop immediately.
                let transition = rt.begin_initialize().ok();
                if transition.is_some() {
                    successes.fetch_add(1, Ordering::SeqCst);
                }

                attempted.wait();
                release.wait();

                drop(transition);
            }));
        }

        // All threads have started, wait for them to attempt.
        start.wait();
        attempted.wait();

        // At this point all guards are still alive, so exactly one
        // CAS must have succeeded.
        assert_eq!(successes.load(Ordering::SeqCst), 1);

        // Release all guards.
        release.wait();
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(rt.state(), RuntimeState::Uninitialized);
    }

    #[test]
    #[ignore = "requires multiple hardware threads — run manually or in CI with sufficient cores"]
    fn acquire_and_shutdown_have_one_winner() {
        let rt = Arc::new(RuntimeLifecycle::new());
        init(&rt);

        for _ in 0..10 {
            // Ensure clean state each iteration.
            rt.word
                .store(encode(RuntimeState::Running, 0), Ordering::Release);

            let start = Arc::new(Barrier::new(3)); // 2 threads + main
            let attempted = Arc::new(Barrier::new(3));
            let release = Arc::new(Barrier::new(3));

            let acquired = Arc::new(AtomicUsize::new(0));
            let shutdown = Arc::new(AtomicUsize::new(0));

            let acquire_thread = {
                let rt = Arc::clone(&rt);
                let start = Arc::clone(&start);
                let attempted = Arc::clone(&attempted);
                let release = Arc::clone(&release);
                let acquired = Arc::clone(&acquired);

                thread::spawn(move || {
                    start.wait();

                    // Hold the lease — do NOT drop until release.
                    let _lease = rt.acquire().ok();
                    if _lease.is_some() {
                        acquired.store(1, Ordering::SeqCst);
                    }

                    attempted.wait();
                    release.wait();
                })
            };

            let shutdown_thread = {
                let rt = Arc::clone(&rt);
                let start = Arc::clone(&start);
                let attempted = Arc::clone(&attempted);
                let release = Arc::clone(&release);
                let shutdown = Arc::clone(&shutdown);

                thread::spawn(move || {
                    start.wait();

                    let _transition = rt.begin_shutdown().ok();
                    if _transition.is_some() {
                        shutdown.store(1, Ordering::SeqCst);
                    }

                    attempted.wait();
                    release.wait();
                })
            };

            start.wait();
            attempted.wait();

            assert_eq!(
                acquired.load(Ordering::SeqCst) + shutdown.load(Ordering::SeqCst),
                1
            );

            release.wait();
            acquire_thread.join().unwrap();
            shutdown_thread.join().unwrap();

            // Both guards/leases dropped → Running, count 0.
            assert_eq!(rt.state(), RuntimeState::Running);
            assert_eq!(rt.active_objects(), 0);
        }
    }

    // ---- compile-time: lease is not Clone ----

    #[allow(dead_code)]
    fn lease_is_not_clone(_lease: RuntimeLease<'_>) {
        // The following line would fail to compile:
        // let _copy = lease.clone();
    }
}
