//! Mock semaphore implementation.
//!
//! Uses `Rc<RefCell<CountingSemaphoreState>>` for shared ownership.
//! BinarySemaphore delegates to CountingSemaphore(max=1, initial=0).
//!
//! # Capability boundary
//!
//! - Core contracts: supported
//! - Blocking contracts: deferred (single-context model)
//!
//! # Timeout semantics
//!
//! - `Timeout::NoWait` / `After(_)`: succeeds if count>0, else Timeout
//! - `Timeout::Forever`: succeeds if count>0, else Unsupported

use alloc::rc::Rc;
use core::cell::RefCell;

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::semaphore::{BinarySemaphore, CountingSemaphore};

use osal_portable::counting_semaphore::CountingSemaphoreState;

// ---------------------------------------------------------------------------
// MockCountingSemaphore
// ---------------------------------------------------------------------------

/// A mock counting semaphore for contract testing.
#[derive(Clone)]
pub struct MockCountingSemaphore {
    inner: Rc<RefCell<CountingSemaphoreState>>,
}

impl MockCountingSemaphore {
    pub fn new(max_count: u32, initial_count: u32) -> Result<Self> {
        Ok(Self {
            inner: Rc::new(RefCell::new(CountingSemaphoreState::new(
                max_count,
                initial_count,
            )?)),
        })
    }
}

impl CountingSemaphore for MockCountingSemaphore {
    fn new(max_count: u32, initial_count: u32) -> Result<Self> {
        Self::new(max_count, initial_count)
    }

    fn acquire(&self, timeout: Timeout) -> Result<()> {
        if self.inner.borrow_mut().try_acquire() {
            return Ok(());
        }
        // count == 0 — cannot satisfy in single-context model
        match timeout {
            Timeout::NoWait => Err(Error::Timeout),
            Timeout::After(_) => Err(Error::Timeout),
            Timeout::Forever => Err(Error::Unsupported),
        }
    }

    fn release(&self) -> Result<()> {
        self.inner.borrow_mut().release()
    }

    fn max_count(&self) -> u32 {
        self.inner.borrow().max_count()
    }

    fn count(&self) -> Result<u32> {
        Ok(self.inner.borrow().count())
    }
}

// ---------------------------------------------------------------------------
// MockBinarySemaphore
// ---------------------------------------------------------------------------

/// A mock binary semaphore. Delegates to [`MockCountingSemaphore`]
/// with `max_count = 1`, `initial_count = 0`.
#[derive(Clone)]
pub struct MockBinarySemaphore {
    inner: MockCountingSemaphore,
}

impl MockBinarySemaphore {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: MockCountingSemaphore::new(1, 0)?,
        })
    }
}

impl BinarySemaphore for MockBinarySemaphore {
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

/// Factory for creating mock semaphores.
pub struct MockSemaphoreFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::SemaphoreFactory for MockSemaphoreFactory {
    type CountingSemaphore = MockCountingSemaphore;
    type BinarySemaphore = MockBinarySemaphore;

    fn create_counting_semaphore(&self, max: u32, initial: u32) -> Result<Self::CountingSemaphore> {
        MockCountingSemaphore::new(max, initial)
    }

    fn create_binary_semaphore(&self) -> Result<Self::BinarySemaphore> {
        MockBinarySemaphore::new()
    }
}
