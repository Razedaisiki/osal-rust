//! POSIX queue implementation.
//!
//! Wraps [`ByteQueue`] with `pthread_mutex_t` + `pthread_cond_t` for
//! thread-safe access, implementing the [`Queue`] trait.

use alloc::sync::Arc;
use core::cell::UnsafeCell;

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue;

use osal_portable::byte_queue::ByteQueue;
use osal_shared::validation;

use crate::sys::condvar::PosixCondvar;
use crate::sys::mutex::PosixMutex;

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct QueueInner {
    mutex: PosixMutex,
    not_empty: PosixCondvar,
    not_full: PosixCondvar,
    buffer: UnsafeCell<ByteQueue>,
    closed: UnsafeCell<bool>,
}

// Safety: all access to buffer/closed is protected by mutex.
unsafe impl Send for QueueInner {}
unsafe impl Sync for QueueInner {}

// ---------------------------------------------------------------------------
// Public type
// ---------------------------------------------------------------------------

/// A POSIX queue backed by pthread mutex + condvar.
///
/// Uses `Arc` internally so cloned handles share the same backend
/// resource.
pub struct PosixQueue {
    inner: Arc<QueueInner>,
}

impl Clone for PosixQueue {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl PosixQueue {
    /// Create a new POSIX queue.
    pub fn new(capacity: usize, msg_size: usize) -> Result<Self> {
        validation::validate_queue_capacity(capacity)?;
        validation::validate_queue_message_size(msg_size)?;
        Ok(Self {
            inner: Arc::new(QueueInner {
                mutex: PosixMutex::new()?,
                not_empty: PosixCondvar::new()?,
                not_full: PosixCondvar::new()?,
                buffer: UnsafeCell::new(ByteQueue::new(capacity, msg_size)?),
                closed: UnsafeCell::new(false),
            }),
        })
    }

    fn buffer(&self) -> &mut ByteQueue {
        unsafe { &mut *self.inner.buffer.get() }
    }

    fn is_closed(&self) -> bool {
        unsafe { *self.inner.closed.get() }
    }

    fn set_closed(&self) {
        unsafe { *self.inner.closed.get() = true; }
    }
}

// ---------------------------------------------------------------------------
// Queue trait
// ---------------------------------------------------------------------------

impl Queue for PosixQueue {
    fn new(capacity: usize, msg_size: usize) -> Result<Self> {
        Self::new(capacity, msg_size)
    }

    fn send(&self, data: &[u8], timeout: Timeout) -> Result<()> {
        self.inner.mutex.lock()?;
        let result = if self.is_closed() {
            Err(Error::QueueClosed)
        } else {
            match timeout {
                Timeout::NoWait => self.buffer().try_send(data),
                Timeout::After(_) | Timeout::Forever => {
                    match self.buffer().try_send(data) {
                        Err(Error::QueueFull) => Err(Error::Timeout),
                        other => other,
                    }
                }
            }
        };
        if result.is_ok() {
            let _ = self.inner.not_empty.signal();
        }
        self.inner.mutex.unlock()?;
        result
    }

    fn recv(&self, buffer: &mut [u8], timeout: Timeout) -> Result<()> {
        self.inner.mutex.lock()?;
        let result = match timeout {
            Timeout::NoWait => self.buffer().try_recv(buffer).map(|_| ()),
            Timeout::After(_) | Timeout::Forever => {
                match self.buffer().try_recv(buffer) {
                    Err(Error::QueueEmpty) => Err(Error::Timeout),
                    other => other.map(|_| ()),
                }
            }
        };
        if result.is_ok() {
            let _ = self.inner.not_full.signal();
        }
        self.inner.mutex.unlock()?;
        result
    }

    fn close(&self) {
        self.inner.mutex.lock().ok();
        self.set_closed();
        self.buffer().close();
        let _ = self.inner.not_empty.broadcast();
        let _ = self.inner.not_full.broadcast();
        self.inner.mutex.unlock().ok();
    }

    fn isr_send(&self, data: &[u8]) -> Result<()> {
        self.send(data, Timeout::NoWait)
    }

    fn isr_recv(&self, buffer: &mut [u8]) -> Result<()> {
        self.recv(buffer, Timeout::NoWait)
    }

    fn capacity(&self) -> usize {
        self.inner.mutex.lock().ok();
        let c = self.buffer().capacity();
        self.inner.mutex.unlock().ok();
        c
    }

    fn msg_size(&self) -> usize {
        self.inner.mutex.lock().ok();
        let s = self.buffer().message_size();
        self.inner.mutex.unlock().ok();
        s
    }

    fn len(&self) -> usize {
        self.inner.mutex.lock().ok();
        let l = self.buffer().len();
        self.inner.mutex.unlock().ok();
        l
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

/// Factory for creating POSIX queues.
pub struct PosixQueueFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::QueueFactory for PosixQueueFactory {
    type Queue = PosixQueue;

    fn create_queue(&self, capacity: usize, msg_size: usize) -> Result<Self::Queue> {
        PosixQueue::new(capacity, msg_size)
    }
}
