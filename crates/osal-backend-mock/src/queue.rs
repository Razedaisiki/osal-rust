//! Mock queue implementation with optional fault injection.
//!
//! Wraps [`ByteQueue`] in `Rc<RefCell<>>` for shared ownership,
//! implementing the [`Queue`] trait for contract testing.
//!
//! # Fault injection
//!
//! When created via [`MockQueue::new_with_faults`], the queue consults
//! a shared [`FaultState`] before each send. One-shot: each fault
//! triggers once, then clears.
//!
//! # Timeout semantics
//!
//! - `Timeout::NoWait`: immediate try_send / try_recv.
//! - `Timeout::After(_)`: maps `QueueFull`/`QueueEmpty` →
//!   `Error::Timeout` (no real waiting).
//! - `Timeout::Forever`: succeeds if ready; returns `Error::Unsupported`
//!   if the operation would block.

use alloc::rc::Rc;
use core::cell::RefCell;

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue;

use osal_portable::byte_queue::ByteQueue;
use osal_shared::validation;

use crate::fault::FaultState;
use crate::wait::apply_timeout;

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct MockQueueInner {
    buffer: ByteQueue,
    faults: Option<Rc<RefCell<FaultState>>>,
}

impl MockQueueInner {
    fn new(capacity: usize, msg_size: usize, faults: Option<Rc<RefCell<FaultState>>>) -> Result<Self> {
        validation::validate_queue_capacity(capacity)?;
        validation::validate_queue_message_size(msg_size)?;
        Ok(Self {
            buffer: ByteQueue::new(capacity, msg_size)?,
            faults,
        })
    }
}

// ---------------------------------------------------------------------------
// Public type
// ---------------------------------------------------------------------------

/// A mock queue for contract testing.
///
/// Uses `Rc<RefCell<>>` internally so cloned handles share the same
/// backend resource.
pub struct MockQueue {
    inner: Rc<RefCell<MockQueueInner>>,
}

impl Clone for MockQueue {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl MockQueue {
    /// Create a new mock queue without fault injection.
    pub fn new(capacity: usize, msg_size: usize) -> Result<Self> {
        Ok(Self {
            inner: Rc::new(RefCell::new(MockQueueInner::new(capacity, msg_size, None)?)),
        })
    }

    /// Create a new mock queue with fault injection support.
    pub fn new_with_faults(
        capacity: usize,
        msg_size: usize,
        faults: Rc<RefCell<FaultState>>,
    ) -> Result<Self> {
        // Check for injected create fault (one-shot).
        if let Some(fault) = faults.borrow_mut().next_queue_create.take() {
            return Err(fault);
        }
        Ok(Self {
            inner: Rc::new(RefCell::new(MockQueueInner::new(capacity, msg_size, Some(faults))?)),
        })
    }

    /// Try to consume a one-shot send fault. Returns the configured
    /// error if one is pending, otherwise `None`.
    fn take_send_fault(&self) -> Option<Error> {
        self.inner
            .borrow()
            .faults
            .as_ref()
            .and_then(|f| f.borrow_mut().next_queue_send.take())
    }
}

// ---------------------------------------------------------------------------
// Queue trait
// ---------------------------------------------------------------------------

impl Queue for MockQueue {
    fn new(capacity: usize, msg_size: usize) -> Result<Self> {
        Self::new(capacity, msg_size)
    }

    fn send(&self, data: &[u8], timeout: Timeout) -> Result<()> {
        if let Some(fault) = self.take_send_fault() {
            return Err(fault);
        }
        apply_timeout(
            timeout,
            self.inner.borrow_mut().buffer.try_send(data),
            Error::QueueFull,
        )
    }

    fn recv(&self, buffer: &mut [u8], timeout: Timeout) -> Result<()> {
        apply_timeout(
            timeout,
            self.inner.borrow_mut().buffer.try_recv(buffer).map(|_| ()),
            Error::QueueEmpty,
        )
    }

    fn close(&self) -> Result<()> {
        self.inner.borrow_mut().buffer.close();
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.inner.borrow().buffer.capacity()
    }

    fn msg_size(&self) -> usize {
        self.inner.borrow().buffer.message_size()
    }

    fn len(&self) -> Result<usize> {
        Ok(self.inner.borrow().buffer.len())
    }
}

// ---------------------------------------------------------------------------
// Factories (testkit)
// ---------------------------------------------------------------------------

/// Factory for creating mock queues (no fault injection).
pub struct MockQueueFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::QueueFactory for MockQueueFactory {
    type Queue = MockQueue;

    fn create_queue(&self, capacity: usize, msg_size: usize) -> Result<Self::Queue> {
        MockQueue::new(capacity, msg_size)
    }
}

/// Factory with shared fault state for queue + fault contract tests.
pub struct MockFaultyQueueFactory {
    faults: Rc<RefCell<FaultState>>,
}

impl MockFaultyQueueFactory {
    /// Create a new factory with empty fault state.
    pub fn new() -> Self {
        Self {
            faults: Rc::new(RefCell::new(FaultState::default())),
        }
    }
}

#[cfg(feature = "testkit")]
impl osal_testkit::factory::QueueFactory for MockFaultyQueueFactory {
    type Queue = MockQueue;

    fn create_queue(&self, capacity: usize, msg_size: usize) -> Result<Self::Queue> {
        MockQueue::new_with_faults(capacity, msg_size, Rc::clone(&self.faults))
    }
}

#[cfg(feature = "testkit")]
impl osal_testkit::factory::FaultFactory for MockFaultyQueueFactory {
    fn clear_faults(&self) {
        self.faults.borrow_mut().clear();
    }

    fn fail_next_queue_create(&self, error: Error) {
        self.faults.borrow_mut().next_queue_create = Some(error);
    }

    fn fail_next_queue_send(&self, error: Error) {
        self.faults.borrow_mut().next_queue_send = Some(error);
    }
}
