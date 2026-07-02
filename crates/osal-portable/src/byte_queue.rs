//! Fixed-size message buffer with close-drain semantics.
//!
//! `ByteQueue` is a reusable ring buffer for FIFO byte-message storage.
//! It is the shared data structure underlying Mock, POSIX, and FreeRTOS
//! queue implementations.

use alloc::vec::Vec;

use osal_api::error::{Error, Result};

/// A fixed-size, ring-buffer-based message queue.
///
/// All messages have the same byte length (`message_size`). The queue
/// supports close-drain semantics: after `close()`, sends are rejected
/// but already-buffered messages remain readable.
pub struct ByteQueue {
    storage: Vec<u8>,
    capacity: usize,
    message_size: usize,
    head: usize,
    tail: usize,
    len: usize,
    closed: bool,
}

impl ByteQueue {
    /// Create a new queue.
    ///
    /// Returns `Error::InvalidParameter` if `capacity == 0` or
    /// `message_size == 0`. Returns `Error::OutOfMemory` on allocation
    /// failure.
    pub fn new(capacity: usize, message_size: usize) -> Result<Self> {
        if capacity == 0 || message_size == 0 {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            storage: alloc::vec![0u8; capacity * message_size],
            capacity,
            message_size,
            head: 0,
            tail: 0,
            len: 0,
            closed: false,
        })
    }

    /// Maximum number of messages.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Current number of messages in the queue.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Number of free slots.
    pub fn available_slots(&self) -> usize {
        self.capacity - self.len
    }

    /// Fixed byte size of each message.
    pub fn message_size(&self) -> usize {
        self.message_size
    }

    /// `true` if the queue contains no messages.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `true` if the queue is at capacity.
    pub fn is_full(&self) -> bool {
        self.len == self.capacity
    }

    /// `true` if the queue has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Permanently close the queue.
    ///
    /// After close, sends are rejected but buffered messages remain
    /// readable. Idempotent.
    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Non-blocking send.
    ///
    /// Returns `Error::QueueClosed` if the queue is closed.
    /// Returns `Error::QueueFull` if the queue is at capacity.
    /// Returns `Error::InvalidMessageSize` if `message.len()` does not
    /// match `self.message_size()`.
    pub fn try_send(&mut self, message: &[u8]) -> Result<()> {
        if self.closed {
            return Err(Error::QueueClosed);
        }
        if message.len() != self.message_size {
            return Err(Error::InvalidMessageSize);
        }
        if self.is_full() {
            return Err(Error::QueueFull);
        }
        let offset = self.tail * self.message_size;
        self.storage[offset..offset + self.message_size].copy_from_slice(message);
        self.tail = (self.tail + 1) % self.capacity;
        self.len += 1;
        Ok(())
    }

    /// Non-blocking receive.
    ///
    /// Returns `Error::QueueClosed` if the queue is closed **and** empty.
    /// Returns `Error::QueueEmpty` if the queue is open and empty.
    /// Returns `Error::InvalidMessageSize` if `out.len() < self.message_size()`.
    ///
    /// On success, returns the number of bytes written to `out` (always
    /// `self.message_size()`).
    pub fn try_recv(&mut self, out: &mut [u8]) -> Result<usize> {
        if out.len() < self.message_size {
            return Err(Error::InvalidMessageSize);
        }
        if self.len == 0 {
            return if self.closed {
                Err(Error::QueueClosed)
            } else {
                Err(Error::QueueEmpty)
            };
        }
        let offset = self.head * self.message_size;
        out[..self.message_size].copy_from_slice(&self.storage[offset..offset + self.message_size]);
        self.head = (self.head + 1) % self.capacity;
        self.len -= 1;
        Ok(self.message_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_zero_capacity() {
        assert!(ByteQueue::new(0, 4).is_err());
    }

    #[test]
    fn new_rejects_zero_message_size() {
        assert!(ByteQueue::new(4, 0).is_err());
    }

    #[test]
    fn send_recv_one_message() {
        let mut q = ByteQueue::new(4, 4).unwrap();
        q.try_send(&[1, 2, 3, 4]).unwrap();
        let mut buf = [0u8; 4];
        let n = q.try_recv(&mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(buf, [1, 2, 3, 4]);
    }

    #[test]
    fn fifo_order_is_preserved() {
        let mut q = ByteQueue::new(4, 2).unwrap();
        q.try_send(&[1, 2]).unwrap();
        q.try_send(&[3, 4]).unwrap();
        let mut buf = [0u8; 2];
        q.try_recv(&mut buf).unwrap();
        assert_eq!(buf, [1, 2]);
        q.try_recv(&mut buf).unwrap();
        assert_eq!(buf, [3, 4]);
    }

    #[test]
    fn send_rejects_wrong_size() {
        let mut q = ByteQueue::new(4, 4).unwrap();
        assert!(q.try_send(&[1, 2]).is_err());
    }

    #[test]
    fn recv_rejects_small_buffer() {
        let mut q = ByteQueue::new(4, 4).unwrap();
        q.try_send(&[1, 2, 3, 4]).unwrap();
        let mut buf = [0u8; 2];
        assert!(q.try_recv(&mut buf).is_err());
    }

    #[test]
    fn send_fails_when_full() {
        let mut q = ByteQueue::new(1, 2).unwrap();
        q.try_send(&[1, 2]).unwrap();
        assert!(q.try_send(&[3, 4]).is_err());
    }

    #[test]
    fn recv_fails_when_empty() {
        let mut q = ByteQueue::new(4, 2).unwrap();
        let mut buf = [0u8; 2];
        assert!(q.try_recv(&mut buf).is_err());
    }

    #[test]
    fn close_rejects_future_send() {
        let mut q = ByteQueue::new(4, 2).unwrap();
        q.close();
        assert!(matches!(q.try_send(&[1, 2]), Err(Error::QueueClosed)));
    }

    #[test]
    fn close_allows_drain() {
        let mut q = ByteQueue::new(4, 2).unwrap();
        q.try_send(&[1, 2]).unwrap();
        q.try_send(&[3, 4]).unwrap();
        q.close();
        let mut buf = [0u8; 2];
        q.try_recv(&mut buf).unwrap();
        assert_eq!(buf, [1, 2]);
        q.try_recv(&mut buf).unwrap();
        assert_eq!(buf, [3, 4]);
    }

    #[test]
    fn closed_empty_recv_returns_queue_closed() {
        let mut q = ByteQueue::new(4, 2).unwrap();
        q.close();
        let mut buf = [0u8; 2];
        assert!(matches!(q.try_recv(&mut buf), Err(Error::QueueClosed)));
    }
}
