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
#[derive(Debug)]
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
    /// `message_size == 0`.
    /// Returns `Error::Overflow` if `capacity * message_size` overflows
    /// `usize`.
    /// Returns `Error::OutOfMemory` if the storage allocation fails.
    pub fn new(capacity: usize, message_size: usize) -> Result<Self> {
        if capacity == 0 || message_size == 0 {
            return Err(Error::InvalidParameter);
        }

        let storage_size = capacity
            .checked_mul(message_size)
            .ok_or(Error::Overflow)?;

        let mut storage = Vec::new();
        storage
            .try_reserve_exact(storage_size)
            .map_err(|_| Error::OutOfMemory)?;
        storage.resize(storage_size, 0);

        Ok(Self {
            storage,
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
    /// Error precedence (highest first):
    /// 1. `Error::InvalidMessageSize` — `message.len()` does not match
    /// 2. `Error::QueueClosed` — queue is closed
    /// 3. `Error::QueueFull` — queue is at capacity
    pub fn try_send(&mut self, message: &[u8]) -> Result<()> {
        debug_assert!(self.len <= self.capacity);
        debug_assert!(self.head < self.capacity || self.capacity == 0);
        debug_assert!(self.tail < self.capacity || self.capacity == 0);
        debug_assert_eq!(self.storage.len(), self.capacity * self.message_size);

        if message.len() != self.message_size {
            return Err(Error::InvalidMessageSize);
        }
        if self.closed {
            return Err(Error::QueueClosed);
        }
        if self.is_full() {
            return Err(Error::QueueFull);
        }
        let offset = self.tail * self.message_size;
        self.storage[offset..offset + self.message_size].copy_from_slice(message);
        self.tail = (self.tail + 1) % self.capacity;
        self.len += 1;

        debug_assert!(self.len <= self.capacity);
        Ok(())
    }

    /// Non-blocking receive.
    ///
    /// Error precedence (highest first):
    /// 1. `Error::InvalidMessageSize` — `out.len()` does not match
    /// 2. `Error::QueueClosed` — queue is closed **and** empty
    /// 3. `Error::QueueEmpty` — queue is open and empty
    ///
    /// On success, returns the number of bytes written to `out` (always
    /// `self.message_size()`).
    pub fn try_recv(&mut self, out: &mut [u8]) -> Result<usize> {
        debug_assert!(self.len <= self.capacity);
        debug_assert!(self.head < self.capacity || self.capacity == 0);
        debug_assert!(self.tail < self.capacity || self.capacity == 0);
        debug_assert_eq!(self.storage.len(), self.capacity * self.message_size);

        if out.len() != self.message_size {
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
        out[..self.message_size]
            .copy_from_slice(&self.storage[offset..offset + self.message_size]);
        self.head = (self.head + 1) % self.capacity;
        self.len -= 1;

        debug_assert!(self.len <= self.capacity);
        Ok(self.message_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Creation
    // ------------------------------------------------------------------

    #[test]
    fn new_rejects_zero_capacity() {
        assert_eq!(
            ByteQueue::new(0, 4).unwrap_err(),
            Error::InvalidParameter
        );
    }

    #[test]
    fn new_rejects_zero_message_size() {
        assert_eq!(
            ByteQueue::new(4, 0).unwrap_err(),
            Error::InvalidParameter
        );
    }

    #[test]
    fn new_rejects_capacity_overflow() {
        // usize::MAX * 2 overflows on any platform
        assert_eq!(
            ByteQueue::new(usize::MAX, 2).unwrap_err(),
            Error::Overflow
        );
    }

    #[test]
    fn new_succeeds_with_valid_params() {
        let q = ByteQueue::new(8, 16).unwrap();
        assert_eq!(q.capacity(), 8);
        assert_eq!(q.message_size(), 16);
        assert_eq!(q.len(), 0);
        assert!(q.is_empty());
        assert!(!q.is_full());
        assert!(!q.is_closed());
    }

    // ------------------------------------------------------------------
    // Send / Recv round-trip
    // ------------------------------------------------------------------

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
    fn wrap_around() {
        // capacity=2, message_size=2: buffer is 4 bytes
        let mut q = ByteQueue::new(2, 2).unwrap();
        q.try_send(&[1, 2]).unwrap();
        q.try_send(&[3, 4]).unwrap();

        let mut buf = [0u8; 2];
        q.try_recv(&mut buf).unwrap();
        assert_eq!(buf, [1, 2]);

        // This send wraps tail around to index 0
        q.try_send(&[5, 6]).unwrap();

        q.try_recv(&mut buf).unwrap();
        assert_eq!(buf, [3, 4]);

        q.try_recv(&mut buf).unwrap();
        assert_eq!(buf, [5, 6]);
    }

    // ------------------------------------------------------------------
    // Error conditions — precise error variant assertions
    // ------------------------------------------------------------------

    #[test]
    fn send_rejects_wrong_size() {
        let mut q = ByteQueue::new(4, 4).unwrap();
        assert_eq!(
            q.try_send(&[1, 2]).unwrap_err(),
            Error::InvalidMessageSize
        );
    }

    #[test]
    fn recv_rejects_wrong_buffer_size_too_small() {
        let mut q = ByteQueue::new(4, 4).unwrap();
        q.try_send(&[1, 2, 3, 4]).unwrap();
        let mut buf = [0u8; 2];
        assert_eq!(q.try_recv(&mut buf).unwrap_err(), Error::InvalidMessageSize);
    }

    #[test]
    fn recv_rejects_wrong_buffer_size_too_large() {
        let mut q = ByteQueue::new(4, 4).unwrap();
        q.try_send(&[1, 2, 3, 4]).unwrap();
        let mut buf = [0u8; 8];
        assert_eq!(q.try_recv(&mut buf).unwrap_err(), Error::InvalidMessageSize);
    }

    #[test]
    fn send_fails_when_full() {
        let mut q = ByteQueue::new(1, 2).unwrap();
        q.try_send(&[1, 2]).unwrap();
        assert_eq!(q.try_send(&[3, 4]).unwrap_err(), Error::QueueFull);
    }

    #[test]
    fn recv_fails_when_empty() {
        let mut q = ByteQueue::new(4, 2).unwrap();
        let mut buf = [0u8; 2];
        assert_eq!(q.try_recv(&mut buf).unwrap_err(), Error::QueueEmpty);
    }

    // ------------------------------------------------------------------
    // Error precedence
    // ------------------------------------------------------------------

    #[test]
    fn closed_queue_with_wrong_size_returns_invalid_message_size() {
        // Parameter validation takes priority over object state.
        let mut q = ByteQueue::new(4, 4).unwrap();
        q.close();
        assert_eq!(
            q.try_send(&[1, 2]).unwrap_err(),
            Error::InvalidMessageSize
        );
    }

    #[test]
    fn closed_queue_with_wrong_recv_buffer_returns_invalid_message_size() {
        let mut q = ByteQueue::new(4, 4).unwrap();
        q.close();
        let mut buf = [0u8; 2];
        assert_eq!(q.try_recv(&mut buf).unwrap_err(), Error::InvalidMessageSize);
    }

    // ------------------------------------------------------------------
    // Close semantics
    // ------------------------------------------------------------------

    #[test]
    fn close_rejects_future_send() {
        let mut q = ByteQueue::new(4, 2).unwrap();
        q.close();
        assert_eq!(q.try_send(&[1, 2]).unwrap_err(), Error::QueueClosed);
    }

    #[test]
    fn close_is_idempotent() {
        let mut q = ByteQueue::new(4, 2).unwrap();
        q.close();
        q.close(); // must not panic
        assert!(q.is_closed());
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
        assert_eq!(q.try_recv(&mut buf).unwrap_err(), Error::QueueClosed);
    }

    #[test]
    fn send_on_full_queue_then_close_drains_correctly() {
        let mut q = ByteQueue::new(2, 2).unwrap();
        q.try_send(&[1, 2]).unwrap();
        q.try_send(&[3, 4]).unwrap();
        // Queue is full
        assert_eq!(q.try_send(&[5, 6]).unwrap_err(), Error::QueueFull);
        q.close();
        // Drain
        let mut buf = [0u8; 2];
        q.try_recv(&mut buf).unwrap();
        assert_eq!(buf, [1, 2]);
        q.try_recv(&mut buf).unwrap();
        assert_eq!(buf, [3, 4]);
        // Now closed and empty
        assert_eq!(q.try_recv(&mut buf).unwrap_err(), Error::QueueClosed);
    }
}
