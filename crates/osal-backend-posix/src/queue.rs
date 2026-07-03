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
    buffer: UnsafeCell<ByteQueue>,
    closed: UnsafeCell<bool>,
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
                closed: UnsafeCell::new(false),
            }),
        })
    }

    // ---- UnsafeCell accessors (caller must hold the lock) ----------

    fn buffer_locked(&self, _guard: &PosixMutexGuard<'_>) -> &mut ByteQueue {
        unsafe { &mut *self.inner.buffer.get() }
    }

    fn is_closed_locked(&self, _guard: &PosixMutexGuard<'_>) -> bool {
        unsafe { *self.inner.closed.get() }
    }

    fn set_closed_locked(&self, _guard: &PosixMutexGuard<'_>) {
        unsafe {
            *self.inner.closed.get() = true;
        }
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
        validation::validate_send_message_size(self.msg_size(), data.len())?;

        let mut guard = self.inner.mutex.lock_guard()?;

        if self.is_closed_locked(&guard) {
            return Err(Error::QueueClosed);
        }

        match timeout {
            Timeout::NoWait => {
                let result = self.buffer_locked(&guard).try_send(data);
                if result.is_ok() {
                    let _ = self.inner.not_empty.signal();
                }
                result
            }
            Timeout::After(d) => {
                let deadline = condvar::abs_deadline(d);
                loop {
                    if self.is_closed_locked(&guard) {
                        return Err(Error::QueueClosed);
                    }
                    match self.buffer_locked(&guard).try_send(data) {
                        Ok(()) => {
                            let _ = self.inner.not_empty.signal();
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
                if self.is_closed_locked(&guard) {
                    return Err(Error::QueueClosed);
                }
                match self.buffer_locked(&guard).try_send(data) {
                    Ok(()) => {
                        let _ = self.inner.not_empty.signal();
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
        validation::validate_recv_buffer_size(self.msg_size(), buffer.len())?;

        let mut guard = self.inner.mutex.lock_guard()?;

        match timeout {
            Timeout::NoWait => {
                let result = self.buffer_locked(&guard).try_recv(buffer).map(|_| ());
                if result.is_ok() {
                    let _ = self.inner.not_full.signal();
                }
                result
            }
            Timeout::After(d) => {
                let deadline = condvar::abs_deadline(d);
                loop {
                    if self.is_closed_locked(&guard)
                        && self.buffer_locked(&guard).len() == 0
                    {
                        return Err(Error::QueueClosed);
                    }
                    match self.buffer_locked(&guard).try_recv(buffer) {
                        Ok(_) => {
                            let _ = self.inner.not_full.signal();
                            return Ok(());
                        }
                        Err(Error::QueueEmpty) => {
                            if self.is_closed_locked(&guard) {
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
                if self.is_closed_locked(&guard)
                    && self.buffer_locked(&guard).len() == 0
                {
                    return Err(Error::QueueClosed);
                }
                match self.buffer_locked(&guard).try_recv(buffer) {
                    Ok(_) => {
                        let _ = self.inner.not_full.signal();
                        return Ok(());
                    }
                    Err(Error::QueueEmpty) => {
                        if self.is_closed_locked(&guard) {
                            return Err(Error::QueueClosed);
                        }
                        self.inner.not_empty.wait(&mut guard)?;
                    }
                    Err(e) => return Err(e),
                }
            },
        }
    }

    fn close(&self) {
        let Ok(guard) = self.inner.mutex.lock_guard() else {
            return;
        };
        self.set_closed_locked(&guard);
        self.buffer_locked(&guard).close();
        let _ = self.inner.not_empty.broadcast();
        let _ = self.inner.not_full.broadcast();
    }

    fn isr_send(&self, data: &[u8]) -> Result<()> {
        self.send(data, Timeout::NoWait)
    }

    fn isr_recv(&self, buffer: &mut [u8]) -> Result<()> {
        self.recv(buffer, Timeout::NoWait)
    }

    fn capacity(&self) -> usize {
        let Ok(guard) = self.inner.mutex.lock_guard() else {
            return 0;
        };
        self.buffer_locked(&guard).capacity()
    }

    fn msg_size(&self) -> usize {
        let Ok(guard) = self.inner.mutex.lock_guard() else {
            return 0;
        };
        self.buffer_locked(&guard).message_size()
    }

    fn len(&self) -> usize {
        let Ok(guard) = self.inner.mutex.lock_guard() else {
            return 0;
        };
        self.buffer_locked(&guard).len()
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
