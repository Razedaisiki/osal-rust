//! FreeRTOS queue implementation.
//!
//! Uses `ByteQueue` (portable ring buffer) for data and close state,
//! a native FreeRTOS mutex for state protection, and two native counting
//! semaphores for waiter wake signalling (ADR 0027 §1).
//!
//! # Architecture
//!
//! ```text
//! FreeRtosQueue
//!     └── Arc<QueueInner>
//!             ├── RuntimeLease
//!             ├── native mutex (state_mutex)
//!             ├── native counting semaphore (sender_wake)
//!             ├── native counting semaphore (receiver_wake)
//!             └── UnsafeCell<QueueState>
//!                     ├── ByteQueue
//!                     ├── sender_waiters / receiver_waiters
//!                     └── sender_wake_credits / receiver_wake_credits
//! ```
//!
//! # Lock order (ADR 0027 §3)
//!
//! 1. Acquire state_mutex
//! 2. Inspect or mutate QueueState
//! 3. Optionally register as waiter
//! 4. Release state_mutex
//! 5. Block on sender_wake or receiver_wake semaphore
//! 6. Re-acquire state_mutex
//! 7. Confirm credit, unregister waiter, re-check QueueState
//!
//! MUST NOT block on a wake semaphore while holding the state mutex.

use alloc::rc::Rc;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::fmt;
use core::marker::PhantomData;

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue;
use osal_portable::byte_queue::ByteQueue;
use osal_shared::runtime::RuntimeLease;
use osal_shared::validation;

use crate::wait::{self, WaitBudget, WaitOutcome};
use osal_backend_freertos_sys as sys;

// ---------------------------------------------------------------------------
// QueueState — protected by state_mutex
// ---------------------------------------------------------------------------

struct QueueState {
    buffer: ByteQueue,

    /// Number of senders that have registered and not yet exited the
    /// wait protocol.
    sender_waiters: u32,

    /// Number of receivers that have registered and not yet exited the
    /// wait protocol.
    receiver_waiters: u32,

    /// Tokens already posted to sender_wake semaphore but not yet
    /// confirmed by a waiter under the state mutex.
    sender_wake_credits: u32,

    /// Tokens already posted to receiver_wake semaphore but not yet
    /// confirmed by a waiter under the state mutex.
    receiver_wake_credits: u32,
}

// ---------------------------------------------------------------------------
// QueueInner — Arc-shared, owns all native resources
// ---------------------------------------------------------------------------

struct QueueInner {
    state_mutex: Option<sys::MutexHandle>,
    sender_wake: Option<sys::SemaphoreHandle>,
    receiver_wake: Option<sys::SemaphoreHandle>,
    state: UnsafeCell<QueueState>,

    /// Cached construction parameters — lock-free read access.
    capacity: usize,
    message_size: usize,

    /// Held for the lifetime of the queue (ADR 0019 §6).
    _lease: RuntimeLease<'static>,
}

// Safety: native mutex serialises all access to QueueState.
// ByteQueue is not Send on its own (contains Vec<u8>), but the mutex
// guarantees exclusive access.
unsafe impl Send for QueueInner {}
unsafe impl Sync for QueueInner {}

impl Drop for QueueInner {
    fn drop(&mut self) {
        // Defensive: no waiters should be registered at drop time.
        // The application is responsible for ensuring no send/recv
        // call is in flight when the last handle drops.
        debug_assert!(
            unsafe { &*self.state.get() }.sender_waiters == 0,
            "queue dropped with registered sender waiters"
        );
        debug_assert!(
            unsafe { &*self.state.get() }.receiver_waiters == 0,
            "queue dropped with registered receiver waiters"
        );

        // Delete native objects in creation-reverse order.
        if let Some(h) = self.receiver_wake.take() {
            sys::semaphore_delete(h);
        }
        if let Some(h) = self.sender_wake.take() {
            sys::semaphore_delete(h);
        }
        if let Some(h) = self.state_mutex.take() {
            sys::mutex_delete(h);
        }
        // ByteQueue and RuntimeLease drop naturally.
    }
}

// ---------------------------------------------------------------------------
// QueueStateGuard — !Send + !Sync mutex guard wrapper
// ---------------------------------------------------------------------------

/// RAII guard for the queue state mutex.
///
/// Provides `&mut QueueState` access.  Releases the native mutex on drop.
/// `!Send + !Sync` — must not be moved to another task.
struct QueueStateGuard<'a> {
    native: &'a sys::MutexHandle,
    state: &'a mut QueueState,
    _not_send: PhantomData<Rc<()>>,
}

impl QueueStateGuard<'_> {
    fn state_mut(&mut self) -> &mut QueueState {
        self.state
    }
}

impl Drop for QueueStateGuard<'_> {
    fn drop(&mut self) {
        if sys::mutex_give(self.native) != sys::GiveStatus::Ok {
            panic!("FreeRTOS queue state mutex give failed — invariant violation");
        }
    }
}

// ---------------------------------------------------------------------------
// Native resources RAII guard for constructor rollback
// ---------------------------------------------------------------------------

/// Holds partially-created native objects during construction.
/// On successful construction, use [`take`][Self::take] to transfer
/// ownership.  On failure, `Drop` cleans up whatever was created.
struct NativeQueueResources {
    mutex: Option<sys::MutexHandle>,
    sender_wake: Option<sys::SemaphoreHandle>,
    receiver_wake: Option<sys::SemaphoreHandle>,
}

impl NativeQueueResources {
    fn new() -> Self {
        Self {
            mutex: None,
            sender_wake: None,
            receiver_wake: None,
        }
    }
}

impl Drop for NativeQueueResources {
    fn drop(&mut self) {
        if let Some(h) = self.receiver_wake.take() {
            sys::semaphore_delete(h);
        }
        if let Some(h) = self.sender_wake.take() {
            sys::semaphore_delete(h);
        }
        if let Some(h) = self.mutex.take() {
            sys::mutex_delete(h);
        }
    }
}

// ---------------------------------------------------------------------------
// FreeRtosQueue — public type
// ---------------------------------------------------------------------------

/// A bounded FIFO queue of fixed-size byte messages.
///
/// Uses `Arc<QueueInner>` internally; cloned handles share the same
/// underlying queue (ADR 0006).
///
/// # Drop constraint
///
/// The application must ensure no `send()` or `recv()` call is in
/// flight when the last handle drops. Dropping while a task is
/// blocked on a wake semaphore results in undefined behaviour.
pub struct FreeRtosQueue {
    inner: Arc<QueueInner>,
}

impl Clone for FreeRtosQueue {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl fmt::Debug for FreeRtosQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FreeRtosQueue")
            .field("capacity", &self.inner.capacity)
            .field("message_size", &self.inner.message_size)
            .finish()
    }
}

impl FreeRtosQueue {
    // ------------------------------------------------------------------
    // Construction (ADR 0027 §7)
    // ------------------------------------------------------------------

    /// Create a new queue.
    ///
    /// Constructor order:
    /// 1. Validate `capacity` and `msg_size`
    /// 2. Create `ByteQueue` (Rust allocation — fail early)
    /// 3. Acquire `RuntimeLease`
    /// 4. Create native state mutex
    /// 5. Create sender wake semaphore (max = capacity, initial = 0)
    /// 6. Create receiver wake semaphore (max = capacity, initial = 0)
    /// 7. Construct `Arc<QueueInner>`
    pub fn new(capacity: usize, msg_size: usize) -> Result<Self> {
        // 1. Validate parameters.
        validation::validate_queue_capacity(capacity)?;
        validation::validate_queue_message_size(msg_size)?;
        // capacity * msg_size overflow is checked by ByteQueue::new.

        // 2. Create ByteQueue — fail before any native allocation.
        let buffer = ByteQueue::new(capacity, msg_size)?;

        // 3. Acquire runtime lease.
        let lease = crate::runtime::acquire_object()?;

        // 4-6. Create native objects with RAII rollback.
        let mut res = NativeQueueResources::new();

        let state_mutex = sys::mutex_create().ok_or(Error::OutOfMemory)?;
        res.mutex = Some(state_mutex);

        // Wake semaphores are signalling channels, not capacity trackers.
        // Their max_count must accommodate all possible waiters (close
        // broadcasts wake every registered waiter).  Use the native max
        // clamped to the semaphore count range.
        let wake_max = sys::max_semaphore_count().min(u32::MAX as u64) as u32;

        let sender_wake =
            sys::counting_semaphore_create(wake_max, 0).ok_or(Error::OutOfMemory)?;
        res.sender_wake = Some(sender_wake);

        let receiver_wake =
            sys::counting_semaphore_create(wake_max, 0).ok_or(Error::OutOfMemory)?;
        res.receiver_wake = Some(receiver_wake);

        // 7. Success — transfer ownership out of the RAII guard.
        let state_mutex = res.mutex.take().unwrap();
        let sender_wake = res.sender_wake.take().unwrap();
        let receiver_wake = res.receiver_wake.take().unwrap();
        // NativeQueueResources::drop runs here with empty Options — no-op.

        Ok(Self {
            inner: Arc::new(QueueInner {
                state_mutex: Some(state_mutex),
                sender_wake: Some(sender_wake),
                receiver_wake: Some(receiver_wake),
                state: UnsafeCell::new(QueueState {
                    buffer,
                    sender_waiters: 0,
                    receiver_waiters: 0,
                    sender_wake_credits: 0,
                    receiver_wake_credits: 0,
                }),
                capacity,
                message_size: msg_size,
                _lease: lease,
            }),
        })
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Acquire the state mutex, returning a guard that provides
    /// `&mut QueueState` access.
    ///
    /// Uses an optimistic zero-tick attempt first; falls back to
    /// blocking only if there is genuine contention (ADR 0027 §4).
    fn lock_state(&self) -> Result<QueueStateGuard<'_>> {
        let state_mutex = self
            .inner
            .state_mutex
            .as_ref()
            .expect("queue already deleted");

        match sys::mutex_take(state_mutex, 0) {
            sys::TakeStatus::Acquired => { /* acquired without blocking */ }
            sys::TakeStatus::Timeout => {
                // Contention — wait indefinitely for the internal mutex.
                // Queue timeout governs the Queue operation, not this
                // internal lock.
                wait::wait_native(Timeout::Forever, |ticks| {
                    sys::mutex_take(state_mutex, ticks)
                })?;
            }
            sys::TakeStatus::Invalid => {
                panic!("FreeRTOS queue state mutex invalid on live queue")
            }
        }

        // SAFETY: we hold the native mutex — exclusive access guaranteed.
        let state_ref = unsafe { &mut *self.inner.state.get() };

        Ok(QueueStateGuard {
            native: state_mutex,
            state: state_ref,
            _not_send: PhantomData,
        })
    }

    /// Signal one receiver if any are waiting with unconsumed credits.
    ///
    /// MUST be called while holding the state mutex.
    fn signal_one_receiver(state: &mut QueueState, receiver_wake: &sys::SemaphoreHandle) {
        if state.receiver_waiters > state.receiver_wake_credits {
            match sys::semaphore_give(receiver_wake) {
                sys::GiveStatus::Ok => {
                    state.receiver_wake_credits += 1;
                }
                sys::GiveStatus::Full | sys::GiveStatus::Invalid => {
                    panic!(
                        "FreeRTOS queue: receiver wake semaphore give failed — \
                         invariant violation"
                    );
                }
            }
        }
    }

    /// Signal one sender if any are waiting with unconsumed credits.
    ///
    /// MUST be called while holding the state mutex.
    fn signal_one_sender(state: &mut QueueState, sender_wake: &sys::SemaphoreHandle) {
        if state.sender_waiters > state.sender_wake_credits {
            match sys::semaphore_give(sender_wake) {
                sys::GiveStatus::Ok => {
                    state.sender_wake_credits += 1;
                }
                sys::GiveStatus::Full | sys::GiveStatus::Invalid => {
                    panic!(
                        "FreeRTOS queue: sender wake semaphore give failed — \
                         invariant violation"
                    );
                }
            }
        }
    }

    /// Re-acquire the state mutex and unregister a sender waiter.
    ///
    /// Used when [`WaitBudget::wait_once`] returns an error (e.g.
    /// scheduler NotStarted) after a sender has already been registered.
    fn unregister_sender_waiter(
        inner: &QueueInner,
        state_mutex: &sys::MutexHandle,
    ) {
        // Re-acquire state mutex.
        loop {
            match sys::mutex_take(state_mutex, 0) {
                sys::TakeStatus::Acquired => break,
                sys::TakeStatus::Timeout => {
                    // Contention — block until available.
                    if wait::wait_native(Timeout::Forever, |ticks| {
                        sys::mutex_take(state_mutex, ticks)
                    })
                    .is_err()
                    {
                        continue;
                    }
                    break;
                }
                sys::TakeStatus::Invalid => {
                    panic!("FreeRTOS queue state mutex invalid on live queue")
                }
            }
        }
        // SAFETY: we hold the state mutex.
        let state = unsafe { &mut *inner.state.get() };
        state.sender_waiters = state.sender_waiters.saturating_sub(1);
        // Release the mutex.
        if sys::mutex_give(state_mutex) != sys::GiveStatus::Ok {
            panic!("FreeRTOS queue state mutex give failed — invariant violation");
        }
    }

    /// Re-acquire the state mutex and unregister a receiver waiter.
    fn unregister_receiver_waiter(
        inner: &QueueInner,
        state_mutex: &sys::MutexHandle,
    ) {
        loop {
            match sys::mutex_take(state_mutex, 0) {
                sys::TakeStatus::Acquired => break,
                sys::TakeStatus::Timeout => {
                    if wait::wait_native(Timeout::Forever, |ticks| {
                        sys::mutex_take(state_mutex, ticks)
                    })
                    .is_err()
                    {
                        continue;
                    }
                    break;
                }
                sys::TakeStatus::Invalid => {
                    panic!("FreeRTOS queue state mutex invalid on live queue")
                }
            }
        }
        // SAFETY: we hold the state mutex.
        let state = unsafe { &mut *inner.state.get() };
        state.receiver_waiters = state.receiver_waiters.saturating_sub(1);
        if sys::mutex_give(state_mutex) != sys::GiveStatus::Ok {
            panic!("FreeRTOS queue state mutex give failed — invariant violation");
        }
    }

    /// Wake ALL registered waiters in a given direction.
    ///
    /// Uses the missing-credit count to avoid double-posting tokens
    /// that have already been emitted but not yet consumed (ADR 0027 §2).
    ///
    /// MUST be called while holding the state mutex.
    fn broadcast_wake(
        waiters: u32,
        wake_credits: &mut u32,
        wake_handle: &sys::SemaphoreHandle,
        direction: &str,
    ) {
        let missing = waiters.saturating_sub(*wake_credits);
        for _ in 0..missing {
            match sys::semaphore_give(wake_handle) {
                sys::GiveStatus::Ok => {}
                sys::GiveStatus::Full | sys::GiveStatus::Invalid => {
                    panic!(
                        "FreeRTOS queue: {direction} wake semaphore give failed \
                         during close broadcast — invariant violation"
                    );
                }
            }
        }
        *wake_credits = waiters;
    }
}

// ---------------------------------------------------------------------------
// Queue trait implementation
// ---------------------------------------------------------------------------

impl Queue for FreeRtosQueue {
    fn new(capacity: usize, msg_size: usize) -> Result<Self> {
        Self::new(capacity, msg_size)
    }

    // ------------------------------------------------------------------
    // send
    // ------------------------------------------------------------------

    fn send(&self, data: &[u8], timeout: Timeout) -> Result<()> {
        // 1. Parameter validation (highest priority — ADR 0027 §6).
        validation::validate_send_message_size(self.inner.message_size, data.len())?;

        let sender_wake = self
            .inner
            .sender_wake
            .as_ref()
            .expect("queue already deleted");
        let receiver_wake = self
            .inner
            .receiver_wake
            .as_ref()
            .expect("queue already deleted");

        let mut budget = WaitBudget::new(timeout);

        loop {
            let mut state_guard = self.lock_state()?;
            let state = state_guard.state_mut();

            // Check closed before attempting send.
            if state.buffer.is_closed() {
                return Err(Error::QueueClosed);
            }

            match state.buffer.try_send(data) {
                Ok(()) => {
                    Self::signal_one_receiver(state, receiver_wake);
                    return Ok(());
                }
                Err(Error::QueueFull) => {
                    match budget {
                        WaitBudget::NoWait => return Err(Error::QueueFull),
                        WaitBudget::Zero => return Err(Error::Timeout),
                        WaitBudget::Finite { .. } | WaitBudget::Forever => {
                            // Register as waiter and block.
                            state.sender_waiters += 1;
                        }
                    }
                }
                Err(e) => return Err(e), // QueueClosed already checked above
            }

            // Release state mutex before blocking.
            let state_mutex_for_reacquire = state_guard.native;
            drop(state_guard);

            // Block on sender_wake semaphore.
            let outcome = match budget.wait_once(|ticks| sys::semaphore_take(sender_wake, ticks)) {
                Ok(o) => o,
                Err(e) => {
                    // Roll back waiter registration on error
                    // (e.g. scheduler NotStarted / Busy).
                    Self::unregister_sender_waiter(
                        &self.inner,
                        state_mutex_for_reacquire,
                    );
                    return Err(e);
                }
            };

            // Re-acquire state mutex.
            let mut state_guard = loop {
                match sys::mutex_take(state_mutex_for_reacquire, 0) {
                    sys::TakeStatus::Acquired => {
                        let state_ref = unsafe { &mut *self.inner.state.get() };
                        break QueueStateGuard {
                            native: state_mutex_for_reacquire,
                            state: state_ref,
                            _not_send: PhantomData,
                        };
                    }
                    sys::TakeStatus::Timeout => {
                        wait::wait_native(Timeout::Forever, |ticks| {
                            sys::mutex_take(state_mutex_for_reacquire, ticks)
                        })?;
                    }
                    sys::TakeStatus::Invalid => {
                        panic!("FreeRTOS queue state mutex invalid on live queue")
                    }
                }
            };
            let state = state_guard.state_mut();

            match outcome {
                WaitOutcome::Acquired => {
                    // Confirm the credit.
                    if state.sender_wake_credits > 0 {
                        state.sender_wake_credits -= 1;
                    }
                    // Unregister waiter.
                    state.sender_waiters = state.sender_waiters.saturating_sub(1);
                    // Loop back to re-check ByteQueue (may have been
                    // closed while we were waiting, or another sender
                    // may have filled the slot).
                }
                WaitOutcome::Unavailable => {
                    // Timeout — reconcile race with close/wake.
                    // Check for a token that arrived between our timeout
                    // and re-acquiring the mutex.
                    let had_race_token =
                        sys::semaphore_take(sender_wake, 0) == sys::TakeStatus::Acquired;
                    if had_race_token && state.sender_wake_credits > 0 {
                        state.sender_wake_credits -= 1;
                    }
                    state.sender_waiters = state.sender_waiters.saturating_sub(1);

                    if state.buffer.is_closed() {
                        return Err(Error::QueueClosed);
                    }

                    return Err(Error::Timeout);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // recv
    // ------------------------------------------------------------------

    fn recv(&self, buffer: &mut [u8], timeout: Timeout) -> Result<()> {
        // 1. Parameter validation (highest priority — ADR 0027 §6).
        validation::validate_recv_buffer_size(self.inner.message_size, buffer.len())?;

        let sender_wake = self
            .inner
            .sender_wake
            .as_ref()
            .expect("queue already deleted");
        let receiver_wake = self
            .inner
            .receiver_wake
            .as_ref()
            .expect("queue already deleted");

        let mut budget = WaitBudget::new(timeout);

        loop {
            let mut state_guard = self.lock_state()?;
            let state = state_guard.state_mut();

            match state.buffer.try_recv(buffer) {
                Ok(_) => {
                    Self::signal_one_sender(state, sender_wake);
                    return Ok(());
                }
                Err(Error::QueueEmpty) => {
                    // Queue is empty — check if closed.
                    if state.buffer.is_closed() {
                        return Err(Error::QueueClosed);
                    }
                    match budget {
                        WaitBudget::NoWait => return Err(Error::QueueEmpty),
                        WaitBudget::Zero => return Err(Error::Timeout),
                        WaitBudget::Finite { .. } | WaitBudget::Forever => {
                            // Register as waiter and block.
                            state.receiver_waiters += 1;
                        }
                    }
                }
                Err(e) => return Err(e),
            }

            // Release state mutex before blocking.
            let state_mutex_for_reacquire = state_guard.native;
            drop(state_guard);

            // Block on receiver_wake semaphore.
            let outcome = match budget.wait_once(|ticks| sys::semaphore_take(receiver_wake, ticks)) {
                Ok(o) => o,
                Err(e) => {
                    // Roll back waiter registration on error.
                    Self::unregister_receiver_waiter(
                        &self.inner,
                        state_mutex_for_reacquire,
                    );
                    return Err(e);
                }
            };

            // Re-acquire state mutex.
            let mut state_guard = loop {
                match sys::mutex_take(state_mutex_for_reacquire, 0) {
                    sys::TakeStatus::Acquired => {
                        let state_ref = unsafe { &mut *self.inner.state.get() };
                        break QueueStateGuard {
                            native: state_mutex_for_reacquire,
                            state: state_ref,
                            _not_send: PhantomData,
                        };
                    }
                    sys::TakeStatus::Timeout => {
                        wait::wait_native(Timeout::Forever, |ticks| {
                            sys::mutex_take(state_mutex_for_reacquire, ticks)
                        })?;
                    }
                    sys::TakeStatus::Invalid => {
                        panic!("FreeRTOS queue state mutex invalid on live queue")
                    }
                }
            };
            let state = state_guard.state_mut();

            match outcome {
                WaitOutcome::Acquired => {
                    // Confirm the credit.
                    if state.receiver_wake_credits > 0 {
                        state.receiver_wake_credits -= 1;
                    }
                    // Unregister waiter.
                    state.receiver_waiters = state.receiver_waiters.saturating_sub(1);
                    // Loop back to re-check ByteQueue.
                }
                WaitOutcome::Unavailable => {
                    // Timeout — reconcile race with close/wake.
                    let had_race_token =
                        sys::semaphore_take(receiver_wake, 0) == sys::TakeStatus::Acquired;
                    if had_race_token && state.receiver_wake_credits > 0 {
                        state.receiver_wake_credits -= 1;
                    }
                    state.receiver_waiters = state.receiver_waiters.saturating_sub(1);

                    if state.buffer.is_closed() && state.buffer.is_empty() {
                        return Err(Error::QueueClosed);
                    }

                    return Err(Error::Timeout);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // close
    // ------------------------------------------------------------------

    fn close(&self) -> Result<()> {
        let sender_wake = self
            .inner
            .sender_wake
            .as_ref()
            .expect("queue already deleted");
        let receiver_wake = self
            .inner
            .receiver_wake
            .as_ref()
            .expect("queue already deleted");

        let mut state_guard = self.lock_state()?;
        let state = state_guard.state_mut();

        if state.buffer.is_closed() {
            // Idempotent.
            return Ok(());
        }

        // Commit close first — ByteQueue::close is infallible.
        state.buffer.close();

        // Wake all registered waiters (ADR 0027 §11).
        Self::broadcast_wake(
            state.sender_waiters,
            &mut state.sender_wake_credits,
            sender_wake,
            "sender",
        );
        Self::broadcast_wake(
            state.receiver_waiters,
            &mut state.receiver_wake_credits,
            receiver_wake,
            "receiver",
        );

        Ok(())
    }

    // ------------------------------------------------------------------
    // Introspection
    // ------------------------------------------------------------------

    fn capacity(&self) -> usize {
        self.inner.capacity
    }

    fn msg_size(&self) -> usize {
        self.inner.message_size
    }

    fn len(&self) -> Result<usize> {
        let state_guard = self.lock_state()?;
        Ok(state_guard.state.buffer.len())
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

/// Factory for creating FreeRTOS queues in contract tests.
#[cfg(feature = "testkit")]
pub struct FreeRtosQueueFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::QueueFactory for FreeRtosQueueFactory {
    type Queue = FreeRtosQueue;

    fn create_queue(&self, capacity: usize, msg_size: usize) -> Result<Self::Queue> {
        FreeRtosQueue::new(capacity, msg_size)
    }
}
