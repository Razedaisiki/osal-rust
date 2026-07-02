//! Minimal close-state flag for queue lifetime management.
//!
//! Used by backends to track whether a queue has been explicitly
//! closed. Idempotent close, open-by-default.

/// A simple close-state flag.
///
/// Starts open. `close()` transitions to closed. Idempotent:
/// calling `close()` multiple times is safe.
#[derive(Debug, Default)]
pub struct CloseFlag {
    closed: bool,
}

impl CloseFlag {
    /// Create a new flag in the open state.
    pub const fn new() -> Self {
        Self { closed: false }
    }

    /// Return `true` if the flag is in the closed state.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Transition to the closed state. Idempotent.
    pub fn close(&mut self) {
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_open() {
        assert!(!CloseFlag::new().is_closed());
    }

    #[test]
    fn close_is_idempotent() {
        let mut f = CloseFlag::new();
        f.close();
        assert!(f.is_closed());
        f.close();
        assert!(f.is_closed());
    }

    #[test]
    fn default_is_open() {
        assert!(!CloseFlag::default().is_closed());
    }
}
