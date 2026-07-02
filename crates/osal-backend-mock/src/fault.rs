//! Fault injection for mock backend testing.
//!
//! Implements [`FaultFactory`] for simulating error conditions.

use alloc::rc::Rc;
use core::cell::RefCell;

use osal_api::error::Error;

// ---------------------------------------------------------------------------
// Fault state
// ---------------------------------------------------------------------------

/// Shared fault injection state.
#[derive(Default)]
pub struct FaultState {
    pub next_queue_create: Option<Error>,
    pub next_queue_send: Option<Error>,
}

impl FaultState {
    /// Clear all pending faults.
    pub fn clear(&mut self) {
        self.next_queue_create = None;
        self.next_queue_send = None;
    }
}

// ---------------------------------------------------------------------------
// FaultFactory implementation
// ---------------------------------------------------------------------------

/// Implements testkit's [`FaultFactory`] backed by shared state.
pub struct MockFaultFactory {
    state: Rc<RefCell<FaultState>>,
}

impl MockFaultFactory {
    /// Create a new fault factory with empty fault state.
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(FaultState::default())),
        }
    }

    /// Return a clone of the shared fault state (for queue integration).
    pub fn state(&self) -> Rc<RefCell<FaultState>> {
        Rc::clone(&self.state)
    }
}

impl osal_testkit::factory::FaultFactory for MockFaultFactory {
    fn clear_faults(&self) {
        self.state.borrow_mut().clear();
    }

    fn fail_next_queue_create(&self, error: Error) {
        self.state.borrow_mut().next_queue_create = Some(error);
    }

    fn fail_next_queue_send(&self, error: Error) {
        self.state.borrow_mut().next_queue_send = Some(error);
    }
}
