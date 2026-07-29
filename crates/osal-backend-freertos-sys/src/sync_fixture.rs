//! Host synchronization fixture for FreeRTOS mutex and semaphore.
//!
//! Uses `std::sync::Mutex` + `Condvar` to simulate real waiter/wake-one
//! behaviour on the host CI.  Only compiled when `test-fixture` is enabled.
//!
//! All state is behind a single global lock because fixture tests are
//! single-threaded (or use `--test-threads=1`).

extern crate std;

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::collections::HashMap;
use std::sync::{Condvar, LazyLock, Mutex};
use std::thread::ThreadId;
use std::time::Duration;
use std::vec::Vec;

use super::{GiveStatus, MutexHandle, SemaphoreHandle, TakeStatus};

// ---------------------------------------------------------------------------
// Virtual tick advance — keeps the fixture clock in sync with timed waits.
// ---------------------------------------------------------------------------

/// Advance the fixture's virtual tick counter by `ticks`, respecting
/// the configured tick width (modulo wrap).
pub(crate) fn advance_virtual_ticks(ticks: u64) {
    use super::{TICK_BITS_FIXTURE, TICK_COUNT_FIXTURE, TICK_OVERFLOW_FIXTURE};
    use core::sync::atomic::Ordering;

    let bits = TICK_BITS_FIXTURE.load(Ordering::Relaxed);
    let modulus: u128 = 1u128 << (bits as u32);

    let current_overflow = TICK_OVERFLOW_FIXTURE.load(Ordering::Relaxed);
    let current_count = TICK_COUNT_FIXTURE.load(Ordering::Relaxed);

    let total: u128 = (current_count as u128)
        .checked_add(ticks as u128)
        .expect("fixture tick overflowed u128");

    let wrap_count = total / modulus;
    let new_count = (total % modulus) as u64;
    let new_overflow = current_overflow
        .checked_add(wrap_count as u64)
        .expect("fixture overflow count overflowed u64");

    TICK_COUNT_FIXTURE.store(new_count, Ordering::Relaxed);
    TICK_OVERFLOW_FIXTURE.store(new_overflow, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Global fixture state
// ---------------------------------------------------------------------------

struct FixtureMutexEntry {
    locked: bool,
    owner: Option<ThreadId>,
    deleted: bool,
    waiters: usize,
    /// Number of threads currently inside a Condvar wait on this mutex.
    blocked_count: usize,
}

struct FixtureSemaphoreEntry {
    count: u32,
    max_count: u32,
    deleted: bool,
    waiters: usize,
    /// Number of threads currently inside a Condvar wait on this semaphore.
    blocked_count: usize,
}

struct FixtureState {
    mutexes: HashMap<usize, FixtureMutexEntry>,
    semaphores: HashMap<usize, FixtureSemaphoreEntry>,
    next_id: usize,
    mutex_create_count: usize,
    mutex_delete_count: usize,
    sem_create_count: usize,
    sem_delete_count: usize,
    take_call_ticks: Vec<u64>,
    give_call_count: usize,
}

impl Default for FixtureState {
    fn default() -> Self {
        Self {
            mutexes: HashMap::new(),
            semaphores: HashMap::new(),
            next_id: 1, // nonzero — opaque handles must not be null
            mutex_create_count: 0,
            mutex_delete_count: 0,
            sem_create_count: 0,
            sem_delete_count: 0,
            take_call_ticks: Vec::new(),
            give_call_count: 0,
        }
    }
}

static FIXTURE: LazyLock<(Mutex<FixtureState>, Condvar)> =
    LazyLock::new(|| (Mutex::new(FixtureState::default()), Condvar::new()));

static MAX_FINITE_WAIT_TICKS: AtomicU64 = AtomicU64::new((1u64 << 32) - 2);
static FAIL_NEXT_MUTEX_CREATE: AtomicBool = AtomicBool::new(false);
static FAIL_NEXT_SEM_CREATE: AtomicBool = AtomicBool::new(false);
/// Count of threads currently inside a Condvar wait (incremented
/// atomically just before the call, decremented just after).
pub(super) static BLOCKED_COUNT: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Handle tagging
// ---------------------------------------------------------------------------

fn id_from_mutex_handle(h: &MutexHandle) -> usize {
    h.raw.as_ptr() as usize
}
fn id_from_semaphore_handle(h: &SemaphoreHandle) -> usize {
    h.raw.as_ptr() as usize
}
fn make_mutex_handle(id: usize) -> MutexHandle {
    MutexHandle {
        raw: unsafe { core::ptr::NonNull::new_unchecked(id as *mut core::ffi::c_void) },
    }
}
fn make_semaphore_handle(id: usize) -> SemaphoreHandle {
    SemaphoreHandle {
        raw: unsafe { core::ptr::NonNull::new_unchecked(id as *mut core::ffi::c_void) },
    }
}

// ---------------------------------------------------------------------------
// Mutex fixture
// ---------------------------------------------------------------------------

pub fn mutex_create() -> Option<MutexHandle> {
    if FAIL_NEXT_MUTEX_CREATE.swap(false, Ordering::Relaxed) {
        return None;
    }
    let (lock, _cvar) = &*FIXTURE;
    let mut state = lock.lock().unwrap();
    let id = state.next_id;
    state.next_id += 1;
    state.mutex_create_count += 1;
    state.mutexes.insert(
        id,
        FixtureMutexEntry {
            locked: false,
            owner: None,
            deleted: false,
            waiters: 0,
            blocked_count: 0,
        },
    );
    Some(make_mutex_handle(id))
}

pub fn mutex_take(handle: &MutexHandle, ticks: u64) -> TakeStatus {
    let id = id_from_mutex_handle(handle);
    let current_thread = std::thread::current().id();
    let max_finite = MAX_FINITE_WAIT_TICKS.load(Ordering::Relaxed);

    let (lock, cvar) = &*FIXTURE;

    // Record the tick value.
    {
        let mut state = lock.lock().unwrap();
        state.take_call_ticks.push(ticks);
    }

    loop {
        let mut state = lock.lock().unwrap();
        let entry = state.mutexes.get_mut(&id).expect("mutex not found");
        if entry.deleted {
            return TakeStatus::Invalid;
        }

        if !entry.locked {
            entry.locked = true;
            entry.owner = Some(current_thread);
            return TakeStatus::Acquired;
        }

        if entry.owner == Some(current_thread) {
            // Non-recursive: same thread re-lock fails immediately.
            // Advance ticks so the wait engine's deadline loop terminates.
            if ticks > 0 {
                advance_virtual_ticks(ticks);
            }
            return TakeStatus::Timeout;
        }

        if ticks == 0 {
            return TakeStatus::Timeout;
        }

        // From this point on, ticks > 0. All Timeout returns must
        // advance virtual ticks so the wait engine's deadline loop
        // sees time progress and eventually terminates.

        // Determine wait duration for this attempt.
        let wait_ticks = ticks.min(max_finite);

        // Track waiters so tests can verify blocking.
        entry.waiters += 1;

        // Wait with timeout.
        let timeout = Duration::from_micros((wait_ticks as u128 * 1_000_000 / 1000) as u64);

        // Increment blocked count RIGHT before wait_timeout so that
        // pollers never see the count elevated before the thread is
        // actually inside the Condvar (avoids lost-wakeup races).
        entry.blocked_count += 1;
        BLOCKED_COUNT.fetch_add(1, Ordering::Relaxed);
        let (_state, wait_result) = cvar.wait_timeout(state, timeout).unwrap();
        BLOCKED_COUNT.fetch_sub(1, Ordering::Relaxed);
        state = _state;

        // Re-fetch entry after wait and decrement.
        let entry = state.mutexes.get_mut(&id).unwrap();
        entry.blocked_count = entry.blocked_count.saturating_sub(1);
        entry.waiters = entry.waiters.saturating_sub(1);

        if wait_result.timed_out() {
            // Re-check one last time.
            if !entry.locked {
                entry.locked = true;
                entry.owner = Some(current_thread);
                advance_virtual_ticks(wait_ticks);
                return TakeStatus::Acquired;
            }
            advance_virtual_ticks(wait_ticks);
            return TakeStatus::Timeout;
        }
        // Spurious wakeup — advance ticks and re-loop.
        advance_virtual_ticks(wait_ticks);
    }
}

pub fn mutex_give(handle: &MutexHandle) -> GiveStatus {
    let id = id_from_mutex_handle(handle);
    let current_thread = std::thread::current().id();
    let (lock, cvar) = &*FIXTURE;

    let mut state = lock.lock().unwrap();
    state.give_call_count += 1;

    let entry = state.mutexes.get_mut(&id).expect("mutex not found");
    if entry.deleted {
        return GiveStatus::Invalid;
    }
    if !entry.locked || entry.owner != Some(current_thread) {
        return GiveStatus::Invalid;
    }

    entry.locked = false;
    entry.owner = None;
    cvar.notify_all();
    GiveStatus::Ok
}

pub fn mutex_delete(handle: MutexHandle) {
    let id = id_from_mutex_handle(&handle);
    let (lock, _cvar) = &*FIXTURE;
    let mut state = lock.lock().unwrap();
    state.mutex_delete_count += 1;
    if let Some(entry) = state.mutexes.get(&id) {
        assert!(!entry.locked, "cannot delete a held mutex");
        assert!(!entry.deleted, "mutex already deleted");
    }
    state.mutexes.remove(&id);
}

// ---------------------------------------------------------------------------
// Semaphore fixture
// ---------------------------------------------------------------------------

pub fn counting_semaphore_create(max: u32, initial: u32) -> Option<SemaphoreHandle> {
    if FAIL_NEXT_SEM_CREATE.swap(false, Ordering::Relaxed) {
        return None;
    }
    let (lock, _cvar) = &*FIXTURE;
    let mut state = lock.lock().unwrap();
    let id = state.next_id;
    state.next_id += 1;
    state.sem_create_count += 1;
    state.semaphores.insert(
        id,
        FixtureSemaphoreEntry {
            count: initial,
            max_count: max,
            deleted: false,
            waiters: 0,
            blocked_count: 0,
        },
    );
    Some(make_semaphore_handle(id))
}

pub fn binary_semaphore_create() -> Option<SemaphoreHandle> {
    if FAIL_NEXT_SEM_CREATE.swap(false, Ordering::Relaxed) {
        return None;
    }
    let (lock, _cvar) = &*FIXTURE;
    let mut state = lock.lock().unwrap();
    let id = state.next_id;
    state.next_id += 1;
    state.sem_create_count += 1;
    state.semaphores.insert(
        id,
        FixtureSemaphoreEntry {
            count: 0,
            max_count: 1,
            deleted: false,
            waiters: 0,
            blocked_count: 0,
        },
    );
    Some(make_semaphore_handle(id))
}

pub fn semaphore_take(handle: &SemaphoreHandle, ticks: u64) -> TakeStatus {
    let id = id_from_semaphore_handle(handle);
    let max_finite = MAX_FINITE_WAIT_TICKS.load(Ordering::Relaxed);
    let (lock, cvar) = &*FIXTURE;

    {
        let mut state = lock.lock().unwrap();
        state.take_call_ticks.push(ticks);
    }

    loop {
        let mut state = lock.lock().unwrap();
        let entry = state.semaphores.get_mut(&id).expect("semaphore not found");
        if entry.deleted {
            return TakeStatus::Invalid;
        }

        if entry.count > 0 {
            entry.count -= 1;
            return TakeStatus::Acquired;
        }

        if ticks == 0 {
            return TakeStatus::Timeout;
        }

        let wait_ticks = ticks.min(max_finite);
        let timeout = Duration::from_micros((wait_ticks as u128 * 1_000_000 / 1000) as u64);

        // Track waiters so tests can verify blocking.
        entry.waiters += 1;

        // Increment blocked count RIGHT before wait_timeout.
        entry.blocked_count += 1;
        BLOCKED_COUNT.fetch_add(1, Ordering::Relaxed);
        let (_state, wait_result) = cvar.wait_timeout(state, timeout).unwrap();
        BLOCKED_COUNT.fetch_sub(1, Ordering::Relaxed);
        state = _state;

        // Re-fetch entry after wait and decrement.
        let entry = state.semaphores.get_mut(&id).unwrap();
        entry.blocked_count = entry.blocked_count.saturating_sub(1);
        entry.waiters = entry.waiters.saturating_sub(1);

        if wait_result.timed_out() {
            if entry.count > 0 {
                entry.count -= 1;
                advance_virtual_ticks(wait_ticks);
                return TakeStatus::Acquired;
            }
            advance_virtual_ticks(wait_ticks);
            return TakeStatus::Timeout;
        }
        // Spurious wakeup — advance ticks and re-loop.
        advance_virtual_ticks(wait_ticks);
    }
}

pub fn semaphore_give(handle: &SemaphoreHandle) -> GiveStatus {
    let id = id_from_semaphore_handle(handle);
    let (lock, cvar) = &*FIXTURE;

    let mut state = lock.lock().unwrap();
    state.give_call_count += 1;

    let entry = state.semaphores.get_mut(&id).expect("semaphore not found");
    if entry.deleted {
        return GiveStatus::Invalid;
    }
    if entry.count >= entry.max_count {
        return GiveStatus::Full;
    }

    entry.count += 1;
    cvar.notify_all();
    GiveStatus::Ok
}

pub fn semaphore_count(handle: &SemaphoreHandle) -> u64 {
    let id = id_from_semaphore_handle(handle);
    let (lock, _cvar) = &*FIXTURE;
    let state = lock.lock().unwrap();
    state
        .semaphores
        .get(&id)
        .map(|e| e.count as u64)
        .unwrap_or(0)
}

pub fn semaphore_delete(handle: SemaphoreHandle) {
    let id = id_from_semaphore_handle(&handle);
    let (lock, _cvar) = &*FIXTURE;
    let mut state = lock.lock().unwrap();
    state.sem_delete_count += 1;
    state.semaphores.remove(&id);
}

// ---------------------------------------------------------------------------
// Fixture control API
// ---------------------------------------------------------------------------

pub fn sync_reset() {
    FAIL_NEXT_MUTEX_CREATE.store(false, Ordering::Relaxed);
    FAIL_NEXT_SEM_CREATE.store(false, Ordering::Relaxed);
    MAX_FINITE_WAIT_TICKS.store((1u64 << 32) - 2, Ordering::Relaxed);

    let (lock, _cvar) = &*FIXTURE;
    let mut state = match lock.lock() {
        Ok(g) => g,
        Err(e) => {
            lock.clear_poison();
            e.into_inner()
        }
    };
    state.mutexes.clear();
    state.semaphores.clear();
    state.next_id = 1;
    state.mutex_create_count = 0;
    state.mutex_delete_count = 0;
    state.sem_create_count = 0;
    state.sem_delete_count = 0;
    state.take_call_ticks.clear();
    state.give_call_count = 0;

    // Defensive: no thread should be inside a Condvar wait at reset time.
    // Allow non-zero blocked count — internal tasks are joined by
    // task_fixture_reset (which calls internal_task_fixture_reset) before
    // this function runs.
}

/// Notify all sync object condition variables.  Used to unblock internal
/// task threads (e.g. timer worker) before joining them during reset.
pub fn sync_notify_all() {
    let (_lock, cvar) = &*FIXTURE;
    cvar.notify_all();
}

pub fn sync_set_fail_next_mutex_create(fail: bool) {
    FAIL_NEXT_MUTEX_CREATE.store(fail, Ordering::Relaxed);
}

pub fn sync_set_fail_next_semaphore_create(fail: bool) {
    FAIL_NEXT_SEM_CREATE.store(fail, Ordering::Relaxed);
}

pub fn sync_set_max_finite_wait_ticks(ticks: u64) {
    assert!(ticks >= 2, "max_finite_wait_ticks must be >= 2");
    MAX_FINITE_WAIT_TICKS.store(ticks, Ordering::Relaxed);
}

pub fn sync_mutex_create_count() -> usize {
    let (lock, _cvar) = &*FIXTURE;
    lock.lock().unwrap().mutex_create_count
}
pub fn sync_mutex_delete_count() -> usize {
    let (lock, _cvar) = &*FIXTURE;
    lock.lock().unwrap().mutex_delete_count
}
pub fn sync_sem_create_count() -> usize {
    let (lock, _cvar) = &*FIXTURE;
    lock.lock().unwrap().sem_create_count
}
pub fn sync_sem_delete_count() -> usize {
    let (lock, _cvar) = &*FIXTURE;
    lock.lock().unwrap().sem_delete_count
}
pub fn sync_take_call_ticks() -> Vec<u64> {
    let (lock, _cvar) = &*FIXTURE;
    lock.lock().unwrap().take_call_ticks.clone()
}
pub fn sync_clear_take_call_ticks() {
    let (lock, _cvar) = &*FIXTURE;
    lock.lock().unwrap().take_call_ticks.clear();
}
pub fn sync_give_call_count() -> usize {
    let (lock, _cvar) = &*FIXTURE;
    lock.lock().unwrap().give_call_count
}

#[allow(dead_code)]
pub fn sync_mutex_waiters(handle: &MutexHandle) -> usize {
    let id = id_from_mutex_handle(handle);
    let (lock, _cvar) = &*FIXTURE;
    lock.lock()
        .unwrap()
        .mutexes
        .get(&id)
        .map(|e| e.waiters)
        .unwrap_or(0)
}

#[allow(dead_code)]
pub fn sync_semaphore_waiters(handle: &SemaphoreHandle) -> usize {
    let id = id_from_semaphore_handle(handle);
    let (lock, _cvar) = &*FIXTURE;
    lock.lock()
        .unwrap()
        .semaphores
        .get(&id)
        .map(|e| e.waiters)
        .unwrap_or(0)
}

/// Number of threads currently blocked on this specific mutex's Condvar.
///
/// Unlike the global [`BLOCKED_COUNT`], this tracks per-object blocked
/// count — essential for Queue tests where sender and receiver wait on
/// different semaphores.
pub fn sync_mutex_blocked_count(handle: &MutexHandle) -> usize {
    let id = id_from_mutex_handle(handle);
    let (lock, _cvar) = &*FIXTURE;
    lock.lock()
        .unwrap()
        .mutexes
        .get(&id)
        .map(|e| e.blocked_count)
        .unwrap_or(0)
}

/// Number of threads currently blocked on this specific semaphore's Condvar.
///
/// Unlike the global [`BLOCKED_COUNT`], this tracks per-object blocked
/// count — essential for Queue tests where sender and receiver wait on
/// different semaphores.
pub fn sync_semaphore_blocked_count(handle: &SemaphoreHandle) -> usize {
    let id = id_from_semaphore_handle(handle);
    let (lock, _cvar) = &*FIXTURE;
    lock.lock()
        .unwrap()
        .semaphores
        .get(&id)
        .map(|e| e.blocked_count)
        .unwrap_or(0)
}
