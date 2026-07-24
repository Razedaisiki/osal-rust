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
// Global fixture state
// ---------------------------------------------------------------------------

struct FixtureMutexEntry {
    locked: bool,
    owner: Option<ThreadId>,
    deleted: bool,
}

struct FixtureSemaphoreEntry {
    count: u32,
    max_count: u32,
    deleted: bool,
}

#[derive(Default)]
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

static FIXTURE: LazyLock<(Mutex<FixtureState>, Condvar)> =
    LazyLock::new(|| (Mutex::new(FixtureState::default()), Condvar::new()));

static MAX_FINITE_WAIT_TICKS: AtomicU64 = AtomicU64::new((1u64 << 32) - 2);
static FAIL_NEXT_MUTEX_CREATE: AtomicBool = AtomicBool::new(false);
static FAIL_NEXT_SEM_CREATE: AtomicBool = AtomicBool::new(false);

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
            return TakeStatus::Timeout;
        }

        if ticks == 0 {
            return TakeStatus::Timeout;
        }

        // Determine wait duration for this attempt.
        let wait_ticks = ticks.min(max_finite);

        // Wait with timeout.
        let timeout = Duration::from_micros(
            (wait_ticks as u128 * 1_000_000 / 1000) as u64,
        );

        let (_state, wait_result) = cvar.wait_timeout(state, timeout).unwrap();
        state = _state;

        if wait_result.timed_out() {
            // Re-check one last time.
            let entry = state.mutexes.get_mut(&id).unwrap();
            if !entry.locked {
                entry.locked = true;
                entry.owner = Some(current_thread);
                return TakeStatus::Acquired;
            }
            return TakeStatus::Timeout;
        }
        // Spurious wakeup — re-loop.
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
    cvar.notify_one();
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
        let timeout = Duration::from_micros(
            (wait_ticks as u128 * 1_000_000 / 1000) as u64,
        );

        let (_state, wait_result) = cvar.wait_timeout(state, timeout).unwrap();
        state = _state;

        if wait_result.timed_out() {
            let entry = state.semaphores.get_mut(&id).unwrap();
            if entry.count > 0 {
                entry.count -= 1;
                return TakeStatus::Acquired;
            }
            return TakeStatus::Timeout;
        }
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
    cvar.notify_one();
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
    let mut state = lock.lock().unwrap();
    state.mutexes.clear();
    state.semaphores.clear();
    state.next_id = 1;
    state.mutex_create_count = 0;
    state.mutex_delete_count = 0;
    state.sem_create_count = 0;
    state.sem_delete_count = 0;
    state.take_call_ticks.clear();
    state.give_call_count = 0;
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
