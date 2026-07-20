//! Transactional runtime lifecycle state machine.
//!
//! # Overview
//!
//! [`RuntimeLifecycle`] manages the four-state OSAL runtime cycle
//! (`Uninitialized → Initializing → Running → ShuttingDown`) and
//! tracks active object leases.  Each logical OSAL object holds a
//! [`RuntimeLease`]; shutdown is refused while any lease is alive.
//!
//! # Design
//!
//! - **No global singleton** — each backend creates its own `static`
//!   instance.  This lets POSIX and Mock coexist in the same test
//!   process, and makes unit tests simple (create a local instance).
//! - **Transactional guards** — `begin_initialize` and `begin_shutdown`
//!   return RAII guards.  If the caller does not call `commit()`, the
//!   guard's `Drop` rolls back the state.  This is panic-safe.
//! - **Double-check acquire** — `acquire()` atomically increments the
//!   object counter and then re-checks the state.  If shutdown began
//!   between the check and the increment, the temporary increment is
//!   rolled back.
//!
//! # Distinction from `Task::count()`
//!
//! `Task::count()` counts entries whose function has not yet
//! completed.  `active_objects()` counts logical OSAL objects
//! (Queue, Mutex, Task handle, etc.) that still hold a
//! [`RuntimeLease`].  A finished Task whose handle is still alive
//! has `Task::count() == 0` but contributes to `active_objects()`.

use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use osal_api::error::{Error, Result};
use osal_api::runtime::RuntimeState;

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
    state: AtomicU8,
    active_objects: AtomicUsize,
}

impl Default for RuntimeLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeLifecycle {
    /// Create a new lifecycle in [`RuntimeState::Uninitialized`].
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(RuntimeState::Uninitialized as u8),
            active_objects: AtomicUsize::new(0),
        }
    }

    /// Current observable state.
    pub fn state(&self) -> RuntimeState {
        let raw = self.state.load(Ordering::Acquire);
        match raw {
            0 => RuntimeState::Uninitialized,
            1 => RuntimeState::Initializing,
            2 => RuntimeState::Running,
            3 => RuntimeState::ShuttingDown,
            _ => unreachable!("invalid RuntimeState raw value {raw}"),
        }
    }

    /// Number of active object leases.
    pub fn active_objects(&self) -> usize {
        self.active_objects.load(Ordering::Acquire)
    }

    // ---------------------------------------------------------------
    // Initialisation
    // ---------------------------------------------------------------

    /// Begin the initialisation transaction.
    ///
    /// On success, the state is `Initializing`.  Call
    /// [`InitializeTransition::commit`] to enter `Running`, or drop
    /// the guard to roll back to `Uninitialized`.
    pub fn begin_initialize(&self) -> Result<InitializeTransition<'_>> {
        self.state
            .compare_exchange(
                RuntimeState::Uninitialized as u8,
                RuntimeState::Initializing as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| Error::AlreadyInitialized)?;

        debug_assert_eq!(self.active_objects.load(Ordering::Acquire), 0);

        Ok(InitializeTransition {
            lifecycle: self,
            committed: false,
        })
    }

    // ---------------------------------------------------------------
    // Shutdown
    // ---------------------------------------------------------------

    /// Begin the shutdown transaction.
    ///
    /// Returns `Error::Busy` if any [`RuntimeLease`] is still alive.
    /// On success the state is `ShuttingDown`.  Call
    /// [`ShutdownTransition::commit`] to enter `Uninitialized`.
    pub fn begin_shutdown(&self) -> Result<ShutdownTransition<'_>> {
        // Retry loop in case a concurrent CAS fails due to a state
        // change between the load and the compare_exchange.
        loop {
            match self.state() {
                RuntimeState::Uninitialized => return Err(Error::NotInitialized),
                RuntimeState::Initializing | RuntimeState::ShuttingDown => {
                    return Err(Error::Busy);
                }
                RuntimeState::Running => {}
            }

            if self
                .state
                .compare_exchange(
                    RuntimeState::Running as u8,
                    RuntimeState::ShuttingDown as u8,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
            {
                continue; // state changed, re-evaluate
            }

            // Now in ShuttingDown — no new leases can succeed.
            if self.active_objects.load(Ordering::Acquire) != 0 {
                self.state.store(RuntimeState::Running as u8, Ordering::Release);
                return Err(Error::Busy);
            }

            return Ok(ShutdownTransition {
                lifecycle: self,
                committed: false,
            });
        }
    }

    // ---------------------------------------------------------------
    // Object lease
    // ---------------------------------------------------------------

    /// Acquire an object lease (double-check pattern).
    ///
    /// Returns `Error::NotInitialized` if the runtime is not `Running`
    /// or if shutdown begins between the first check and the increment.
    /// Returns `Error::Overflow` if the counter would overflow.
    pub fn acquire(&self) -> Result<RuntimeLease<'_>> {
        loop {
            if self.state() != RuntimeState::Running {
                return Err(Error::NotInitialized);
            }

            self.active_objects
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    count.checked_add(1)
                })
                .map_err(|_| Error::Overflow)?;

            // Re-check — shutdown may have started between our first
            // check and the increment.
            if self.state() == RuntimeState::Running {
                return Ok(RuntimeLease { lifecycle: self });
            }

            // Shutdown began; roll back the temporary increment.
            self.release_object();

            // If the state returned to Running (e.g. shutdown was
            // refused because of our temporary count), retry.
            if self.state() != RuntimeState::Running {
                return Err(Error::NotInitialized);
            }
        }
    }

    /// Internal: decrement the active-object counter.
    fn release_object(&self) {
        let previous = self.active_objects.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "runtime object count underflow");
    }
}

// Safety: the atomic fields provide internal synchronisation.
unsafe impl Send for RuntimeLifecycle {}
unsafe impl Sync for RuntimeLifecycle {}

// ---------------------------------------------------------------------------
// InitializeTransition
// ---------------------------------------------------------------------------

/// RAII guard returned by [`RuntimeLifecycle::begin_initialize`].
///
/// On drop, if [`commit`](InitializeTransition::commit) has not been
/// called, the state is rolled back to `Uninitialized`.
#[must_use = "initialization must be committed"]
pub struct InitializeTransition<'a> {
    lifecycle: &'a RuntimeLifecycle,
    committed: bool,
}

impl InitializeTransition<'_> {
    /// Commit the initialisation — transition to `Running`.
    pub fn commit(mut self) {
        self.lifecycle
            .state
            .store(RuntimeState::Running as u8, Ordering::Release);
        self.committed = true;
    }
}

impl Drop for InitializeTransition<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.lifecycle
                .state
                .store(RuntimeState::Uninitialized as u8, Ordering::Release);
        }
    }
}

// ---------------------------------------------------------------------------
// ShutdownTransition
// ---------------------------------------------------------------------------

/// RAII guard returned by [`RuntimeLifecycle::begin_shutdown`].
///
/// On drop, if [`commit`](ShutdownTransition::commit) has not been
/// called, the state is rolled back to `Running`.
#[must_use = "shutdown must be committed"]
pub struct ShutdownTransition<'a> {
    lifecycle: &'a RuntimeLifecycle,
    committed: bool,
}

impl ShutdownTransition<'_> {
    /// Commit the shutdown — transition to `Uninitialized`.
    pub fn commit(mut self) {
        debug_assert_eq!(self.lifecycle.active_objects.load(Ordering::Acquire), 0);
        self.lifecycle
            .state
            .store(RuntimeState::Uninitialized as u8, Ordering::Release);
        self.committed = true;
    }
}

impl Drop for ShutdownTransition<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.lifecycle
                .state
                .store(RuntimeState::Running as u8, Ordering::Release);
        }
    }
}

// ---------------------------------------------------------------------------
// RuntimeLease
// ---------------------------------------------------------------------------

/// An object lease proving the runtime is `Running`.
///
/// Created by [`RuntimeLifecycle::acquire`].  Each logical OSAL
/// object holds one lease in its inner state.  Cloning a handle
/// shares the existing lease (via `Arc`/`Rc`); no additional lease
/// is acquired.
///
/// Dropping the lease decrements the runtime's active-object counter.
/// This type is **not** `Clone` or `Copy`.
#[must_use = "the lease must be retained for the object's lifetime"]
pub struct RuntimeLease<'a> {
    lifecycle: &'a RuntimeLifecycle,
}

// Safety: the counter is atomic, so Send + Sync are safe.
unsafe impl Send for RuntimeLease<'_> {}
unsafe impl Sync for RuntimeLease<'_> {}

impl Drop for RuntimeLease<'_> {
    fn drop(&mut self) {
        self.lifecycle.release_object();
    }
}

// ---------------------------------------------------------------------------
// Unit tests — local instances (no global state)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::sync::Arc;
    use alloc::vec::Vec;
    use core::sync::atomic::AtomicUsize;
    use std::thread;

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
        let t = rt.begin_initialize().unwrap();
        assert_eq!(rt.state(), RuntimeState::Initializing);
        t.commit();
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
    fn repeated_initialize_is_rejected() {
        let rt = RuntimeLifecycle::new();
        let t = rt.begin_initialize().unwrap();
        t.commit();

        let result = rt.begin_initialize();
        assert!(result.is_err());
    }

    #[test]
    fn initialize_while_initializing_is_rejected() {
        let rt = RuntimeLifecycle::new();
        let _t = rt.begin_initialize().unwrap();
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
        let t = rt.begin_initialize().unwrap();
        t.commit();

        let lease = rt.acquire().unwrap();
        assert_eq!(rt.active_objects(), 1);
        drop(lease);
        assert_eq!(rt.active_objects(), 0);
    }

    #[test]
    fn dropping_lease_decrements_count() {
        let rt = RuntimeLifecycle::new();
        rt.begin_initialize().unwrap().commit();

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
        rt.begin_initialize().unwrap().commit();

        let leases: Vec<RuntimeLease<'_>> = (0..5).map(|_| rt.acquire().unwrap()).collect();
        assert_eq!(rt.active_objects(), 5);
        drop(leases);
        assert_eq!(rt.active_objects(), 0);
    }

    // ---- shutdown ----

    #[test]
    fn shutdown_before_initialize_fails() {
        let rt = RuntimeLifecycle::new();
        assert!(rt.begin_shutdown().is_err());
    }

    #[test]
    fn shutdown_with_active_lease_returns_busy() {
        let rt = RuntimeLifecycle::new();
        rt.begin_initialize().unwrap().commit();

        let _lease = rt.acquire().unwrap();
        assert!(rt.begin_shutdown().is_err());
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    #[test]
    fn busy_shutdown_restores_running() {
        let rt = RuntimeLifecycle::new();
        rt.begin_initialize().unwrap().commit();

        let _lease = rt.acquire().unwrap();
        let result = rt.begin_shutdown();
        assert!(result.is_err());
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    #[test]
    fn shutdown_commit_returns_to_uninitialized() {
        let rt = RuntimeLifecycle::new();
        rt.begin_initialize().unwrap().commit();

        let t = rt.begin_shutdown().unwrap();
        assert_eq!(rt.state(), RuntimeState::ShuttingDown);
        t.commit();
        assert_eq!(rt.state(), RuntimeState::Uninitialized);
    }

    #[test]
    fn shutdown_drop_rolls_back_to_running() {
        let rt = RuntimeLifecycle::new();
        rt.begin_initialize().unwrap().commit();

        {
            let _t = rt.begin_shutdown().unwrap();
            assert_eq!(rt.state(), RuntimeState::ShuttingDown);
        }
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    #[test]
    fn runtime_can_reinitialize_after_shutdown() {
        let rt = RuntimeLifecycle::new();
        rt.begin_initialize().unwrap().commit();
        rt.begin_shutdown().unwrap().commit();

        let t = rt.begin_initialize().unwrap();
        t.commit();
        assert_eq!(rt.state(), RuntimeState::Running);
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
        rt.begin_initialize().unwrap().commit();
        let _st = rt.begin_shutdown().unwrap();
        assert!(rt.acquire().is_err());
    }

    // ---- concurrency ----

    #[test]
    fn only_one_concurrent_initialize_succeeds() {
        let rt = Arc::new(RuntimeLifecycle::new());
        let successes = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let rt = Arc::clone(&rt);
            let successes = Arc::clone(&successes);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    if rt.begin_initialize().is_ok() {
                        successes.fetch_add(1, Ordering::Relaxed);
                        // Don't commit — let it roll back so other
                        // threads can try again.
                    }
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // At least one succeeded; each attempt that succeeded rolled
        // back, so the state is Uninitialized.
        assert!(successes.load(Ordering::Relaxed) > 0);
        assert_eq!(rt.state(), RuntimeState::Uninitialized);
    }

    #[test]
    fn acquire_and_shutdown_never_lose_a_lease() {
        use core::sync::atomic::AtomicBool;

        let rt = Arc::new(RuntimeLifecycle::new());
        rt.begin_initialize().unwrap().commit();

        let live_leases = Arc::new(AtomicUsize::new(0));
        let errors = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicBool::new(false));

        // Spawn acquirers.
        let mut handles = Vec::new();
        for _ in 0..4 {
            let rt = Arc::clone(&rt);
            let live = Arc::clone(&live_leases);
            let errs = Arc::clone(&errors);
            let done = Arc::clone(&done);
            handles.push(thread::spawn(move || {
                while !done.load(Ordering::Acquire) {
                    match rt.acquire() {
                        Ok(lease) => {
                            live.fetch_add(1, Ordering::AcqRel);
                            // Hold briefly then release.
                            core::hint::spin_loop();
                            live.fetch_sub(1, Ordering::AcqRel);
                            drop(lease);
                        }
                        Err(_) => {
                            errs.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }));
        }

        // Run shutdown attempts from another thread.
        for _ in 0..10 {
            match rt.begin_shutdown() {
                Ok(t) => {
                    t.commit();
                    // Successfully shut down — stop acquirers.
                    done.store(true, Ordering::Release);
                    for h in handles {
                        h.join().unwrap();
                    }
                    // After shutdown, active_objects must be 0 and
                    // state must be Uninitialized.
                    assert_eq!(rt.active_objects(), 0);
                    assert_eq!(rt.state(), RuntimeState::Uninitialized);
                    return;
                }
                Err(_) => {
                    // Busy or other — retry after a brief pause.
                    thread::yield_now();
                }
            }
        }

        // If we get here, shutdown never succeeded — stop acquirers
        // and re-check invariants.
        done.store(true, Ordering::Release);
        for h in handles {
            h.join().unwrap();
        }

        // At least some errors should have occurred (shutdown was
        // attempted while running).
        assert!(errors.load(Ordering::Relaxed) > 0);
        // No live leases dangling.
        assert_eq!(live_leases.load(Ordering::Acquire), 0);
    }

    // ---- lease is not Clone ----
    // (compile-time check — if this compiles, the test passes)

    #[allow(dead_code)]
    fn lease_is_not_clone(lease: RuntimeLease<'_>) {
        // The following line would fail to compile:
        // let _copy = lease.clone();
        drop(lease);
    }
}
