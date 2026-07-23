//! POSIX semaphore implementation.
//!
//! Uses `pthread_mutex_t` + `pthread_cond_t` + [`CountingSemaphoreState`]
//! for blocking acquire with monotonic-clock timeouts.
//! BinarySemaphore delegates to CountingSemaphore(max=1, initial=0).

use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::time::Duration;

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::semaphore::{BinarySemaphore, CountingSemaphore};
use osal_shared::runtime::RuntimeLease;

use osal_portable::counting_semaphore::CountingSemaphoreState;

use crate::sys::condvar::{self, PosixCondvar};
use crate::sys::mutex::PosixMutex;

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct PosixCountingSemaphoreInner {
    mutex: PosixMutex,
    condvar: PosixCondvar,
    state: UnsafeCell<CountingSemaphoreState>,
    /// Cached at construction — immutable, no lock needed.
    max_count: u32,
    /// Held for the lifetime of the semaphore (ADR 0019 §6).
    _runtime: RuntimeLease<'static>,
}

// Safety: the mutex ensures exclusive access to state.
unsafe impl Send for PosixCountingSemaphoreInner {}
unsafe impl Sync for PosixCountingSemaphoreInner {}

// ---------------------------------------------------------------------------
// Public type
// ---------------------------------------------------------------------------

/// A counting semaphore backed by pthread mutex + condvar.
#[derive(Clone)]
pub struct PosixCountingSemaphore {
    inner: Arc<PosixCountingSemaphoreInner>,
}

impl PosixCountingSemaphore {
    pub fn new(max_count: u32, initial_count: u32) -> Result<Self> {
        // 1. Validate parameters first (ADR 0001, ADR 0019 §6).
        let state = CountingSemaphoreState::new(max_count, initial_count)?;

        // 2. Acquire a runtime lease.  If this fails, the validated
        //    state is dropped — no resource leaked.
        let runtime = crate::runtime::acquire_object()?;

        // 3. Create native resources.  On failure the lease drops.
        Ok(Self {
            inner: Arc::new(PosixCountingSemaphoreInner {
                mutex: PosixMutex::new()?,
                condvar: PosixCondvar::new()?,
                state: UnsafeCell::new(state),
                max_count,
                _runtime: runtime,
            }),
        })
    }

    /// Run `f` with mutable access to the state, guarded by the caller's lock.
    ///
    /// # Safety
    ///
    /// The caller must hold `self.inner.mutex`. The `_guard` parameter ties
    /// this access to the locked critical section — it exists only to prove
    /// the lock is held; its value is ignored.
    fn with_state_locked<R>(
        &self,
        _guard: &crate::sys::mutex::PosixMutexGuard<'_>,
        f: impl FnOnce(&mut CountingSemaphoreState) -> R,
    ) -> R {
        // Safety: the caller holds self.inner.mutex, which guarantees
        // exclusive access to the UnsafeCell.
        unsafe { f(&mut *self.inner.state.get()) }
    }
}

impl CountingSemaphore for PosixCountingSemaphore {
    fn new(max_count: u32, initial_count: u32) -> Result<Self> {
        Self::new(max_count, initial_count)
    }

    fn acquire(&self, timeout: Timeout) -> Result<()> {
        let mut guard = self.inner.mutex.lock_guard()?;

        if self.with_state_locked(&guard, |state| state.try_acquire()) {
            return Ok(());
        }

        // Count is 0 — apply timeout strategy
        match timeout {
            Timeout::NoWait => Err(Error::Timeout),
            Timeout::After(d) if d == Duration::ZERO => Err(Error::Timeout),
            Timeout::After(d) => {
                let deadline = condvar::abs_deadline(d)?;
                loop {
                    if self.with_state_locked(&guard, |state| state.try_acquire()) {
                        return Ok(());
                    }
                    match self.inner.condvar.timed_wait(&mut guard, &deadline) {
                        Err(Error::Timeout) => return Err(Error::Timeout),
                        Err(e) => return Err(e),
                        Ok(()) => {} // spurious wakeup — re-check
                    }
                }
            }
            Timeout::Forever => loop {
                if self.with_state_locked(&guard, |state| state.try_acquire()) {
                    return Ok(());
                }
                self.inner.condvar.wait(&mut guard)?;
            },
        }
    }

    fn release(&self) -> Result<()> {
        let guard = self.inner.mutex.lock_guard()?;
        match self.with_state_locked(&guard, |state| state.release()) {
            Ok(()) => {
                // Wake ONE waiter — state already committed.
                self.inner.condvar.wake_one_after_commit();
                Ok(())
            }
            Err(Error::Overflow) => Err(Error::Overflow),
            Err(e) => Err(e),
        }
    }

    fn max_count(&self) -> u32 {
        self.inner.max_count
    }

    fn count(&self) -> Result<u32> {
        let guard = self.inner.mutex.lock_guard()?;
        Ok(self.with_state_locked(&guard, |state| state.count()))
    }
}

// ---------------------------------------------------------------------------
// PosixBinarySemaphore
// ---------------------------------------------------------------------------

/// A binary semaphore. Delegates to [`PosixCountingSemaphore`]
/// with `max_count = 1`, `initial_count = 0`.
#[derive(Clone)]
pub struct PosixBinarySemaphore {
    inner: PosixCountingSemaphore,
}

impl PosixBinarySemaphore {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: PosixCountingSemaphore::new(1, 0)?,
        })
    }
}

impl BinarySemaphore for PosixBinarySemaphore {
    fn new() -> Result<Self> {
        Self::new()
    }

    fn acquire(&self, timeout: Timeout) -> Result<()> {
        self.inner.acquire(timeout)
    }

    fn release(&self) -> Result<()> {
        self.inner.release()
    }

    fn is_signaled(&self) -> Result<bool> {
        Ok(self.inner.count()? == 1)
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

/// Factory for creating POSIX semaphores.
pub struct PosixSemaphoreFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::SemaphoreFactory for PosixSemaphoreFactory {
    type CountingSemaphore = PosixCountingSemaphore;
    type BinarySemaphore = PosixBinarySemaphore;

    fn create_counting_semaphore(&self, max: u32, initial: u32) -> Result<Self::CountingSemaphore> {
        PosixCountingSemaphore::new(max, initial)
    }

    fn create_binary_semaphore(&self) -> Result<Self::BinarySemaphore> {
        PosixBinarySemaphore::new()
    }
}
