//! FreeRTOS counting and binary semaphore implementations.
//!
//! Uses native FreeRTOS counting and binary semaphores with the
//! unified wait engine (`wait.rs`).  The kernel count is the sole
//! source of truth — no Rust-side dual count (ADR 0026 §6).
//!
//! # BinarySemaphore
//!
//! `FreeRtosBinarySemaphore` uses `xSemaphoreCreateBinary()`, which
//! creates an unsignaled (count=0) semaphore.  This matches the OSAL
//! contract: `new()` → initial count 0.

use alloc::sync::Arc;

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::semaphore::{BinarySemaphore, CountingSemaphore};
use osal_shared::runtime::RuntimeLease;

use crate::wait::{self, WaitOutcome};
use osal_backend_freertos_sys as sys;

// ---------------------------------------------------------------------------
// Inner state (shared by both semaphore types)
// ---------------------------------------------------------------------------

struct SemaphoreInner {
    native: Option<sys::SemaphoreHandle>,
    /// Immutable after construction — no lock needed for reads.
    max_count: u32,
    /// Held for the lifetime of the semaphore (ADR 0019 §6).
    _lease: RuntimeLease<'static>,
}

impl Drop for SemaphoreInner {
    fn drop(&mut self) {
        if let Some(h) = self.native.take() {
            sys::semaphore_delete(h);
        }
    }
}

// Safety: FreeRTOS handles are safe to share across tasks.
unsafe impl Send for SemaphoreInner {}
unsafe impl Sync for SemaphoreInner {}

// ---------------------------------------------------------------------------
// FreeRtosCountingSemaphore
// ---------------------------------------------------------------------------

/// A counting semaphore backed by a FreeRTOS native counting semaphore.
///
/// `Clone` shares the same kernel object via `Arc`.
pub struct FreeRtosCountingSemaphore {
    inner: Arc<SemaphoreInner>,
}

impl Clone for FreeRtosCountingSemaphore {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl FreeRtosCountingSemaphore {
    /// Create a counting semaphore with the given max and initial count.
    ///
    /// Constructor order (ADR 0019 §6):
    /// 1. Validate parameters
    /// 2. Validate native range
    /// 3. Acquire [`RuntimeLease`]
    /// 4. Create native semaphore
    /// 5. Construct inner
    pub fn new(max_count: u32, initial_count: u32) -> Result<Self> {
        // 1. Validate parameters.
        if max_count == 0 {
            return Err(Error::InvalidParameter);
        }
        if initial_count > max_count {
            return Err(Error::InvalidParameter);
        }

        // 2. Validate native range.
        let max_native = sys::max_semaphore_count();
        if max_count as u64 > max_native {
            return Err(Error::InvalidParameter);
        }

        // 3. Acquire runtime lease.
        let lease = crate::runtime::acquire_object()?;

        // 4. Create native semaphore.
        let handle = sys::counting_semaphore_create(max_count, initial_count)
            .ok_or(Error::OutOfMemory)?;

        // 5. Construct inner.
        Ok(Self {
            inner: Arc::new(SemaphoreInner {
                native: Some(handle),
                max_count,
                _lease: lease,
            }),
        })
    }
}

impl CountingSemaphore for FreeRtosCountingSemaphore {
    fn new(max_count: u32, initial_count: u32) -> Result<Self> {
        Self::new(max_count, initial_count)
    }

    fn acquire(&self, timeout: Timeout) -> Result<()> {
        let native = self
            .inner
            .native
            .as_ref()
            .expect("semaphore already deleted");

        let outcome = wait::wait_native(timeout, |ticks| {
            sys::semaphore_take(native, ticks)
        })?;

        match outcome {
            WaitOutcome::Acquired => Ok(()),
            WaitOutcome::Unavailable => Err(Error::Timeout),
        }
    }

    fn release(&self) -> Result<()> {
        let native = self
            .inner
            .native
            .as_ref()
            .expect("semaphore already deleted");

        match sys::semaphore_give(native) {
            sys::GiveStatus::Ok => {
                // Kernel has already woken one waiter if any.
                Ok(())
            }
            sys::GiveStatus::Full => Err(Error::Overflow),
            sys::GiveStatus::Invalid => {
                panic!("FreeRTOS semaphore give returned Invalid on a live handle")
            }
        }
    }

    fn max_count(&self) -> u32 {
        self.inner.max_count
    }

    fn count(&self) -> Result<u32> {
        let native = self
            .inner
            .native
            .as_ref()
            .expect("semaphore already deleted");

        Ok(sys::semaphore_count(native) as u32)
    }
}

// ---------------------------------------------------------------------------
// FreeRtosBinarySemaphore
// ---------------------------------------------------------------------------

/// A binary semaphore backed by a FreeRTOS native binary semaphore.
///
/// Created via `xSemaphoreCreateBinary()` — count starts at 0
/// (unsignaled), matching the OSAL contract.
///
/// `Clone` shares the same kernel object via `Arc`.
pub struct FreeRtosBinarySemaphore {
    inner: Arc<SemaphoreInner>,
}

impl Clone for FreeRtosBinarySemaphore {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl FreeRtosBinarySemaphore {
    pub fn new() -> Result<Self> {
        // 1. Acquire runtime lease.
        let lease = crate::runtime::acquire_object()?;

        // 2. Create native binary semaphore.
        let handle =
            sys::binary_semaphore_create().ok_or(Error::OutOfMemory)?;

        // 3. Construct inner.
        Ok(Self {
            inner: Arc::new(SemaphoreInner {
                native: Some(handle),
                max_count: 1,
                _lease: lease,
            }),
        })
    }
}

impl BinarySemaphore for FreeRtosBinarySemaphore {
    fn new() -> Result<Self> {
        Self::new()
    }

    fn acquire(&self, timeout: Timeout) -> Result<()> {
        let native = self
            .inner
            .native
            .as_ref()
            .expect("semaphore already deleted");

        let outcome = wait::wait_native(timeout, |ticks| {
            sys::semaphore_take(native, ticks)
        })?;

        match outcome {
            WaitOutcome::Acquired => Ok(()),
            WaitOutcome::Unavailable => Err(Error::Timeout),
        }
    }

    fn release(&self) -> Result<()> {
        let native = self
            .inner
            .native
            .as_ref()
            .expect("semaphore already deleted");

        match sys::semaphore_give(native) {
            sys::GiveStatus::Ok => Ok(()),
            sys::GiveStatus::Full => Err(Error::Overflow),
            sys::GiveStatus::Invalid => {
                panic!("FreeRTOS semaphore give returned Invalid on a live handle")
            }
        }
    }

    fn is_signaled(&self) -> Result<bool> {
        let native = self
            .inner
            .native
            .as_ref()
            .expect("semaphore already deleted");
        Ok(sys::semaphore_count(native) == 1)
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

/// Factory for creating FreeRTOS semaphores in contract tests.
#[cfg(feature = "testkit")]
pub struct FreeRtosSemaphoreFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::SemaphoreFactory for FreeRtosSemaphoreFactory {
    type CountingSemaphore = FreeRtosCountingSemaphore;
    type BinarySemaphore = FreeRtosBinarySemaphore;

    fn create_counting_semaphore(
        &self,
        max: u32,
        initial: u32,
    ) -> Result<Self::CountingSemaphore> {
        FreeRtosCountingSemaphore::new(max, initial)
    }

    fn create_binary_semaphore(&self) -> Result<Self::BinarySemaphore> {
        FreeRtosBinarySemaphore::new()
    }
}
