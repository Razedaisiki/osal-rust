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
        // Check for injected fault first.
        if let Some(fault) = self.take_send_fault() {
            return Err(fault);
        }

        match timeout {
            Timeout::NoWait => self.inner.borrow_mut().buffer.try_send(data),
            Timeout::After(_) => match self.inner.borrow_mut().buffer.try_send(data) {
                Err(Error::QueueFull) => Err(Error::Timeout),
                other => other,
            },
            Timeout::Forever => match self.inner.borrow_mut().buffer.try_send(data) {
                Err(Error::QueueFull) => Err(Error::Unsupported),
                other => other,
            },
        }
    }

    fn recv(&self, buffer: &mut [u8], timeout: Timeout) -> Result<()> {
        match timeout {
            Timeout::NoWait => {
                let _n = self.inner.borrow_mut().buffer.try_recv(buffer)?;
                Ok(())
            }
            Timeout::After(_) => match self.inner.borrow_mut().buffer.try_recv(buffer) {
                Err(Error::QueueEmpty) => Err(Error::Timeout),
                other => other.map(|_| ()),
            },
            Timeout::Forever => match self.inner.borrow_mut().buffer.try_recv(buffer) {
                Err(Error::QueueEmpty) => Err(Error::Unsupported),
                other => other.map(|_| ()),
            },
        }
    }

    fn close(&self) {
        self.inner.borrow_mut().buffer.close();
    }

    fn isr_send(&self, data: &[u8]) -> Result<()> {
        self.send(data, Timeout::NoWait)
    }

    fn isr_recv(&self, buffer: &mut [u8]) -> Result<()> {
        self.recv(buffer, Timeout::NoWait)
    }

    fn capacity(&self) -> usize {
        self.inner.borrow().buffer.capacity()
    }

    fn msg_size(&self) -> usize {
        self.inner.borrow().buffer.message_size()
    }

    fn len(&self) -> usize {
        self.inner.borrow().buffer.len()
    }
}
