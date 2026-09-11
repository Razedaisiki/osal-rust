//! Queue demo — send / receive roundtrip on a bounded byte queue.

use core::fmt;

use osal::prelude::*;

use crate::error::{DemoError, DemoResult, OsalResultExt};
use crate::runtime::with_runtime;

/// Outcome of the queue demo.
pub struct QueueReport {
    /// Messages successfully sent.
    pub sent: u32,
    /// Messages successfully received.
    pub received: u32,
    /// Message count remaining after the roundtrip.
    pub final_len: usize,
}

impl fmt::Display for QueueReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[queue] sent={}\n[queue] received={}\n[queue] final_len={}\n[queue] roundtrip OK",
            self.sent, self.received, self.final_len
        )
    }
}

/// Run the queue demo against the active backend.
pub fn run() -> DemoResult<QueueReport> {
    with_runtime(|| {
        let queue = Queue::new(4, 4).demo_context("queue.create")?;

        queue
            .send(&1u32.to_le_bytes(), Timeout::NoWait)
            .demo_context("queue.send")?;

        let mut buffer = [0u8; 4];
        queue
            .recv(&mut buffer, Timeout::NoWait)
            .demo_context("queue.recv")?;

        let value = u32::from_le_bytes(buffer);
        if value != 1 {
            return Err(DemoError::Check("queue payload mismatch"));
        }

        let final_len = queue.len().demo_context("queue.len")?;
        if final_len != 0 {
            return Err(DemoError::Check("queue not drained after roundtrip"));
        }

        drop(queue);

        Ok(QueueReport {
            sent: 1,
            received: 1,
            final_len,
        })
    })
}
