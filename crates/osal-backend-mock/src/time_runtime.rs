//! Mock time runtime — thread-local virtual clock and timer registry.
//!
//! Uses the pre-advance model. `collect_expired_actions` returns
//! callbacks that must be executed outside the RefCell borrow.

use alloc::vec::Vec;
use core::time::Duration;

use osal_api::traits::timer::TimerCallback;
use osal_portable::timer_state::TimerState;

// ---------------------------------------------------------------------------
// MockTimerKey — (epoch, id) for safe isolation across resets
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MockTimerKey {
    pub epoch: u64,
    pub id: u64,
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

struct MockTimerEntry {
    key: MockTimerKey,
    state: TimerState,
    callback: Option<TimerCallback>,
    creation_order: u64,
    deleted: bool,
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

pub struct MockTimeRuntime {
    now: Duration,
    epoch: u64,
    next_id: u64,
    next_creation_order: u64,
    timers: Vec<MockTimerEntry>,
}

impl MockTimeRuntime {
    pub fn new() -> Self {
        Self {
            now: Duration::ZERO,
            epoch: 1,
            next_id: 1,
            next_creation_order: 0,
            timers: Vec::new(),
        }
    }

    pub fn now(&self) -> Duration {
        self.now
    }

    pub fn reset(&mut self) {
        self.now = Duration::ZERO;
        self.epoch += 1;
        self.next_id = 1;
        self.next_creation_order = 0;
        self.timers.clear();
    }

    /// Advance time without dispatching callbacks.
    pub fn advance_time(&mut self, d: Duration) {
        self.now = self.now.saturating_add(d);
    }

    /// Register a timer. Returns the key for handle operations.
    pub fn register_timer(
        &mut self,
        period: Duration,
        mode: osal_api::types::TimerMode,
        callback: TimerCallback,
    ) -> MockTimerKey {
        let key = MockTimerKey {
            epoch: self.epoch,
            id: self.next_id,
        };
        self.next_id += 1;
        let order = self.next_creation_order;
        self.next_creation_order += 1;
        self.timers.push(MockTimerEntry {
            key,
            state: TimerState::new(period, mode)
                .expect("TimerState::new should be validated by caller"),
            callback: Some(callback),
            creation_order: order,
            deleted: false,
        });
        key
    }

    fn find_mut(&mut self, key: MockTimerKey) -> Option<&mut MockTimerEntry> {
        self.timers.iter_mut().find(|e| e.key == key && !e.deleted)
    }

    pub fn start_timer(&mut self, key: MockTimerKey) {
        let now = self.now;
        if let Some(e) = self.timers.iter_mut().find(|e| e.key == key && !e.deleted) {
            let _ = e.state.start(now);
        }
    }
    pub fn stop_timer(&mut self, key: MockTimerKey) {
        if let Some(e) = self.timers.iter_mut().find(|e| e.key == key && !e.deleted) {
            e.state.stop();
        }
    }
    pub fn reset_timer(&mut self, key: MockTimerKey) {
        let now = self.now;
        if let Some(e) = self.timers.iter_mut().find(|e| e.key == key && !e.deleted) {
            let _ = e.state.reset(now);
        }
    }
    pub fn change_period(&mut self, key: MockTimerKey, new_period: Duration) {
        if let Some(e) = self.timers.iter_mut().find(|e| e.key == key && !e.deleted) {
            let _ = e.state.change_period(new_period);
        }
    }
    pub fn deregister_timer(&mut self, key: MockTimerKey) {
        if let Some(e) = self.find_mut(key) {
            e.deleted = true;
            e.state.stop();
            e.callback = None;
        }
    }

    /// Take ONE expired callback. Returns (key, callback) for the
    /// earliest expired timer. Caller must execute the callback outside
    /// the RefCell borrow, then call `restore_callback`.
    ///
    /// Only one callback is taken at a time so that callbacks can
    /// stop/reset other timers before their callbacks are taken.
    pub fn take_next_expired(&mut self) -> Option<(MockTimerKey, TimerCallback)> {
        let now = self.now;
        let mut best: Option<(usize, Duration, u64)> = None;

        for (i, e) in self.timers.iter().enumerate() {
            if e.deleted || e.callback.is_none() {
                continue;
            }
            if let Some(d) = e.state.deadline() {
                if d <= now {
                    match best {
                        None => best = Some((i, d, e.creation_order)),
                        Some((_, bd, bo)) if d < bd || (d == bd && e.creation_order < bo) => {
                            best = Some((i, d, e.creation_order));
                        }
                        _ => {}
                    }
                }
            }
        }

        let (idx, _, _) = best?;
        let entry = &mut self.timers[idx];
        if !entry.state.advance_on_expiry(now) {
            return None;
        }
        let cb = entry.callback.take().unwrap();
        Some((entry.key, cb))
    }

    /// Restore a callback after execution.
    pub fn restore_callback(&mut self, key: MockTimerKey, callback: TimerCallback) {
        if let Some(entry) = self.timers.iter_mut().find(|e| e.key == key && !e.deleted) {
            if entry.callback.is_none() {
                entry.callback = Some(callback);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Global runtime singleton (one per process)
// ---------------------------------------------------------------------------

static RUNTIME: spin::Mutex<Option<MockTimeRuntime>> = spin::Mutex::new(None);

pub(crate) fn with_runtime<F, R>(f: F) -> R
where
    F: FnOnce(&mut MockTimeRuntime) -> R,
{
    let mut guard = RUNTIME.lock();
    let rt = guard.get_or_insert_with(MockTimeRuntime::new);
    f(rt)
}

pub(crate) fn take_next_expired() -> Option<(MockTimerKey, TimerCallback)> {
    RUNTIME.lock().as_mut().and_then(|rt| rt.take_next_expired())
}

pub(crate) fn restore_callback(key: MockTimerKey, callback: TimerCallback) {
    if let Some(rt) = RUNTIME.lock().as_mut() {
        rt.restore_callback(key, callback);
    }
}

pub(crate) fn reset_runtime() {
    if let Some(rt) = RUNTIME.lock().as_mut() {
        rt.reset();
    }
}