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

use crate::sys::condvar::{self, PosixCondvar};
use crate::sys::mutex::{PosixMutex, PosixMutexGuard};

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct QueueInner {
    mutex: PosixMutex,
    not_empty: PosixCondvar,
    not_full: PosixCondvar,
    /// The ring buffer (the sole source of state including the `closed` flag).
    buffer: UnsafeCell<ByteQueue>,
    /// Cached construction-time values — immutable, no lock needed.
    capacity: usize,
    message_size: usize,
}

unsafe impl Send for QueueInner {}
unsafe impl Sync for QueueInner {}

// ---------------------------------------------------------------------------
// Public type
// ---------------------------------------------------------------------------

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
    pub fn new(capacity: usize, msg_size: usize) -> Result<Self> {
        validation::validate_queue_capacity(capacity)?;
        validation::validate_queue_message_size(msg_size)?;
        Ok(Self {
            inner: Arc::new(QueueInner {
                mutex: PosixMutex::new()?,
                not_empty: PosixCondvar::new()?,
                not_full: PosixCondvar::new()?,
                buffer: UnsafeCell::new(ByteQueue::new(capacity, msg_size)?),
                capacity,
                message_size: msg_size,
            }),
        })
    }

    /// Access the buffer (caller must hold the lock).
    fn buffer_locked(&self, _guard: &PosixMutexGuard<'_>) -> &mut ByteQueue {
        unsafe { &mut *self.inner.buffer.get() }
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
        // Use cached message_size to avoid locking just for validation.
        validation::validate_send_message_size(self.inner.message_size, data.len())?;

        let mut guard = self.inner.mutex.lock_guard()?;

        // ByteQueue's try_send checks InvalidMessageSize before closed,
        // but we already validated the size. Check closed first here
        // so we don't needlessly call try_send.
        if self.buffer_locked(&guard).is_closed() {
            return Err(Error::QueueClosed);
        }

        match timeout {
            Timeout::NoWait => {
                let result = self.buffer_locked(&guard).try_send(data);
                if result.is_ok() {
                    self.inner.not_empty.signal()?;
                }
                result
            }
            Timeout::After(d) => {
                let deadline = condvar::abs_deadline(d);
                loop {
                    if self.buffer_locked(&guard).is_closed() {
                        return Err(Error::QueueClosed);
                    }
                    match self.buffer_locked(&guard).try_send(data) {
                        Ok(()) => {
                            self.inner.not_empty.signal()?;
                            return Ok(());
                        }
                        Err(Error::QueueFull) => {
                            match self.inner.not_full.timed_wait(&mut guard, &deadline) {
                                Err(Error::Timeout) => return Err(Error::Timeout),
                                Err(e) => return Err(e),
                                Ok(()) => {}
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            Timeout::Forever => loop {
                if self.buffer_locked(&guard).is_closed() {
                    return Err(Error::QueueClosed);
                }
                match self.buffer_locked(&guard).try_send(data) {
                    Ok(()) => {
                        self.inner.not_empty.signal()?;
                        return Ok(());
                    }
                    Err(Error::QueueFull) => {
                        self.inner.not_full.wait(&mut guard)?;
                    }
                    Err(e) => return Err(e),
                }
            },
        }
    }

    fn recv(&self, buffer: &mut [u8], timeout: Timeout) -> Result<()> {
        validation::validate_recv_buffer_size(self.inner.message_size, buffer.len())?;

        let mut guard = self.inner.mutex.lock_guard()?;

        match timeout {
            Timeout::NoWait => {
                let result = self.buffer_locked(&guard).try_recv(buffer).map(|_| ());
                if result.is_ok() {
                    self.inner.not_full.signal()?;
                }
                result
            }
            Timeout::After(d) => {
                let deadline = condvar::abs_deadline(d);
                loop {
                    let is_closed = self.buffer_locked(&guard).is_closed();
                    let is_empty = self.buffer_locked(&guard).len() == 0;

                    if is_closed && is_empty {
                        return Err(Error::QueueClosed);
                    }
                    match self.buffer_locked(&guard).try_recv(buffer) {
                        Ok(_) => {
                            self.inner.not_full.signal()?;
                            return Ok(());
                        }
                        Err(Error::QueueEmpty) => {
                            if is_closed {
                                return Err(Error::QueueClosed);
                            }
                            match self.inner.not_empty.timed_wait(&mut guard, &deadline) {
                                Err(Error::Timeout) => return Err(Error::Timeout),
                                Err(e) => return Err(e),
                                Ok(()) => {}
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            Timeout::Forever => loop {
                let is_closed = self.buffer_locked(&guard).is_closed();
                let is_empty = self.buffer_locked(&guard).len() == 0;

                if is_closed && is_empty {
                    return Err(Error::QueueClosed);
                }
                match self.buffer_locked(&guard).try_recv(buffer) {
                    Ok(_) => {
                        self.inner.not_full.signal()?;
                        return Ok(());
                    }
                    Err(Error::QueueEmpty) => {
                        if is_closed {
                            return Err(Error::QueueClosed);
                        }
                        self.inner.not_empty.wait(&mut guard)?;
                    }
                    Err(e) => return Err(e),
                }
            },
        }
    }

    fn close(&self) -> Result<()> {
        let guard = self.inner.mutex.lock_guard()?;
        let buffer = self.buffer_locked(&guard);

        if buffer.is_closed() {
            return Ok(());
        }

        buffer.close();

        // Wake all blocked senders and receivers. If these fail the state
        // has already been committed — the queue is closed regardless.
        // We propagate the error for visibility.
        self.inner.not_empty.broadcast()?;
        self.inner.not_full.broadcast()?;

        Ok(())
    }

    fn capacity(&self) -> usize {
        self.inner.capacity
    }

    fn msg_size(&self) -> usize {
        self.inner.message_size
    }

    fn len(&self) -> Result<usize> {
        let guard = self.inner.mutex.lock_guard()?;
        Ok(self.buffer_locked(&guard).len())
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

pub struct PosixQueueFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::QueueFactory for PosixQueueFactory {
    type Queue = PosixQueue;

    fn create_queue(&self, capacity: usize, msg_size: usize) -> Result<Self::Queue> {
        PosixQueue::new(capacity, msg_size)
    }
}
