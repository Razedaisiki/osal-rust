//! Queue trait — bounded FIFO message queue.
//!
//! See [the behavior contract](../../../../docs/behavior-contract.md#11-queue-contract)
//! for the full behavioral specification.

use crate::error::Result;
use crate::time::Timeout;

/// A bounded FIFO queue of fixed-size byte messages.
///
/// Messages are always `msg_size` bytes. Sending a shorter or longer
/// slice, or receiving into a differently sized buffer, returns
/// `Error::InvalidMessageSize`.
///
/// # FIFO guarantee
///
/// Messages are received in the order they were sent.
///
/// # Close semantics
///
/// Closing prevents future sends. Already queued messages remain
/// readable. A receiver returns `Error::QueueClosed` only when the
/// queue is both closed and empty.
///
/// Blocked senders are woken with `Error::QueueClosed`. Blocked
/// receivers are woken if the queue is empty; otherwise they may
/// continue draining buffered messages.
///
/// [`close`](Queue::close) is idempotent.
///
/// # Examples
///
/// ```ignore
/// use osal::prelude::*;
///
/// let q = Queue::new(8, 4)?;
///
/// // Send two u32 messages
/// q.send(&1u32.to_le_bytes(), Timeout::NoWait)?;
/// q.send(&2u32.to_le_bytes(), Timeout::NoWait)?;
///
/// // Receive them in order
/// let mut buf = [0u8; 4];
/// q.recv(&mut buf, Timeout::NoWait)?;
/// assert_eq!(buf, 1u32.to_le_bytes());
/// ```
pub trait Queue: Sized {
    /// Create a new queue.
    ///
    /// `capacity` is the maximum number of messages. `msg_size` is the
    /// fixed byte length of each message.
    ///
    /// Returns `Error::InvalidParameter` if either argument is zero.
    /// Returns `Error::OutOfMemory` on allocation failure.
    fn new(capacity: usize, msg_size: usize) -> Result<Self>;

    // ---- sending ----

    /// Send a message, blocking according to `timeout`.
    ///
    /// `data.len()` must equal [`msg_size`](Queue::msg_size).
    ///
    /// | Condition | Result |
    /// |-----------|--------|
    /// | Queue not full | `Ok(())` — message enqueued |
    /// | Full + `NoWait` | `Error::QueueFull` |
    /// | Full + `After(d)` | Block; `Error::Timeout` if no space within `d` |
    /// | Full + `Forever` | Block until space available |
    /// | Queue closed | `Error::QueueClosed` |
    /// | Wrong data length | `Error::InvalidMessageSize` |
    fn send(&self, data: &[u8], timeout: Timeout) -> Result<()>;

    /// Non-blocking send, safe to call from ISR context.
    fn isr_send(&self, data: &[u8]) -> Result<()>;

    // ---- receiving ----

    /// Receive a message, blocking according to `timeout`.
    ///
    /// `buffer.len()` must equal [`msg_size`](Queue::msg_size).
    ///
    /// | Condition | Result |
    /// |-----------|--------|
    /// | Queue not empty | `Ok(())` — oldest message copied into `buffer` |
    /// | Empty + `NoWait` | `Error::QueueEmpty` |
    /// | Empty + `After(d)` | Block; `Error::Timeout` if no message within `d` |
    /// | Empty + `Forever` | Block until message available |
    /// | Queue closed and empty | `Error::QueueClosed` |
    /// | Wrong buffer length | `Error::InvalidMessageSize` |
    fn recv(&self, buffer: &mut [u8], timeout: Timeout) -> Result<()>;

    /// Non-blocking receive, safe to call from ISR context.
    fn isr_recv(&self, buffer: &mut [u8]) -> Result<()>;

    // ---- lifecycle ----

    /// Permanently close the queue.
    ///
    /// Closing prevents future sends. Already queued messages remain
    /// readable. A receiver returns `Error::QueueClosed` only when
    /// the queue is both closed and empty.
    ///
    /// Blocked senders are woken with `Error::QueueClosed`. Blocked
    /// receivers are woken if the queue is empty; otherwise they may
    /// continue draining buffered messages.
    ///
    /// Idempotent: calling `close` on an already-closed queue is safe.
    fn close(&self);

    // ---- introspection ----

    /// Maximum number of messages the queue can hold.
    fn capacity(&self) -> usize;

    /// Fixed byte size of each message.
    fn msg_size(&self) -> usize;

    /// Current number of messages in the queue.
    fn len(&self) -> usize;

    /// `true` if the queue contains no messages.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `true` if the queue is at capacity.
    fn is_full(&self) -> bool {
        self.len() == self.capacity()
    }
}
