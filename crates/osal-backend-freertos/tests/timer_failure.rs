//! FreeRTOS Timer failure-injection and rollback tests.
//!
//! Deterministic Virtual mode.

#![cfg(feature = "testkit")]

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use core::time::Duration;
use std::sync::Arc;

use osal_api::error::Error;
use osal_api::traits::timer::Timer;
use osal_api::types::TimerMode;
use osal_backend_freertos::runtime;
use osal_backend_freertos::timer::{
    FreeRtosTimer, fixture_fail_next_registry_reserve, fixture_registry_len,
    fixture_set_next_timer_id, fixture_worker_exists, flush_timer_service, timer_flush_request,
};
use osal_backend_freertos_sys::fixture;
use osal_backend_freertos_sys::fixture::FixtureWaitMode;

// ---------------------------------------------------------------------------
// TestGuard
// ---------------------------------------------------------------------------

struct TestGuard;

impl TestGuard {
    fn new() -> Self {
        match runtime::shutdown() {
            Ok(()) | Err(Error::NotInitialized) => {}
            Err(e) => panic!("dirty test setup: {e:?}"),
        }
        fixture::reset();
        fixture::set_wait_mode(FixtureWaitMode::Virtual);
        runtime::initialize().expect("initialize");
        TestGuard
    }
}

impl Drop for TestGuard {
    fn drop(&mut self) {
        let target = timer_flush_request();
        flush_timer_service(target);
        match runtime::shutdown() {
            Ok(()) | Err(Error::NotInitialized) => {}
            Err(e) => panic!("dirty test teardown: {e:?}"),
        }
        fixture::set_wait_mode(FixtureWaitMode::Realtime);
    }
}

fn advance_ms(ms: u64) {
    fixture::advance_ticks(ms);
    let target = timer_flush_request();
    flush_timer_service(target);
}

// ---------------------------------------------------------------------------
// 1. Worker creation failure — timer stays stopped, can retry
// ---------------------------------------------------------------------------

#[test]
fn worker_create_failure_leaves_timer_stopped_and_retryable() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    let timer = FreeRtosTimer::new(
        "wf",
        Duration::from_millis(10),
        TimerMode::OneShot,
        Box::new(move || {
            f.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();

    let len_before = fixture_registry_len();

    fixture::set_fail_next_internal_task_create(true);
    let err = timer.start().unwrap_err();
    assert_eq!(err, Error::OutOfMemory);
    assert!(!fixture_worker_exists());
    assert_eq!(fixture::active_internal_task_count(), 0);
    assert_eq!(fixture_registry_len(), len_before);

    // Retry should succeed.
    assert!(timer.start().is_ok());
    advance_ms(10);
    assert_eq!(fired.load(Ordering::Relaxed), 1);
}

// ---------------------------------------------------------------------------
// 2. Registry reserve failure — no leak
// ---------------------------------------------------------------------------

struct DropProbe(Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn registry_reserve_failure_rolls_back_callback_and_runtime_lease() {
    let _guard = TestGuard::new();
    let drop_count = Arc::new(AtomicUsize::new(0));
    let probe = DropProbe(Arc::clone(&drop_count));

    let len_before = fixture_registry_len();

    fixture_fail_next_registry_reserve();
    let result = FreeRtosTimer::new(
        "rf",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            let _p = &probe;
        }),
    );
    assert_eq!(result.err().unwrap(), Error::OutOfMemory);

    // Probe must have been dropped exactly once.
    assert_eq!(drop_count.load(Ordering::Relaxed), 1);
    // Registry must not have an orphaned entry.
    assert_eq!(fixture_registry_len(), len_before);
    // Runtime can shut down cleanly.
}

// ---------------------------------------------------------------------------
// 3. Timer ID overflow
// ---------------------------------------------------------------------------

#[test]
fn timer_id_overflow_preserves_registry_and_drops_callback() {
    let _guard = TestGuard::new();
    let drop_count = Arc::new(AtomicUsize::new(0));
    let probe = DropProbe(Arc::clone(&drop_count));
    let len_before = fixture_registry_len();

    fixture_set_next_timer_id(u64::MAX);
    let result = FreeRtosTimer::new(
        "of",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            let _p = &probe;
        }),
    );
    assert_eq!(result.err().unwrap(), Error::Overflow);

    assert_eq!(drop_count.load(Ordering::Relaxed), 1);
    assert_eq!(fixture_registry_len(), len_before);
}

// ---------------------------------------------------------------------------
// 4. Runtime init: mutex create failure
// ---------------------------------------------------------------------------

#[test]
fn initialize_mutex_failure_is_atomic_and_retryable() {
    fixture::reset();
    fixture::set_fail_next_mutex_create(true);
    let err = runtime::initialize().unwrap_err();
    assert_eq!(err, Error::OutOfMemory);
    assert_eq!(fixture::active_internal_task_count(), 0);

    fixture::set_fail_next_mutex_create(false);
    runtime::initialize().expect("second init should succeed");
    assert!(runtime::shutdown().is_ok());
}

// ---------------------------------------------------------------------------
// 5. Runtime init: semaphore create failure
// ---------------------------------------------------------------------------

#[test]
fn initialize_semaphore_failure_deletes_created_mutex() {
    fixture::reset();
    let mutex_before = fixture::mutex_create_count();
    let mutex_del_before = fixture::mutex_delete_count();

    fixture::set_fail_next_semaphore_create(true);
    let err = runtime::initialize().unwrap_err();
    assert_eq!(err, Error::OutOfMemory);

    // The mutex created during init must have been deleted on rollback.
    assert_eq!(
        fixture::mutex_create_count() - mutex_before,
        fixture::mutex_delete_count() - mutex_del_before
    );

    fixture::set_fail_next_semaphore_create(false);
    runtime::initialize().expect("second init should succeed");
    assert!(runtime::shutdown().is_ok());
}

// ---------------------------------------------------------------------------
// 6. Runtime init: EventGroup create failure
// ---------------------------------------------------------------------------

#[test]
fn initialize_event_group_failure_deletes_mutex_and_semaphore() {
    fixture::reset();
    let mtx_created = fixture::mutex_create_count();
    let mtx_deleted = fixture::mutex_delete_count();
    let sem_created = fixture::sem_create_count();
    let sem_deleted = fixture::sem_delete_count();

    fixture::set_fail_next_event_group_create(true);
    let err = runtime::initialize().unwrap_err();
    assert_eq!(err, Error::OutOfMemory);

    // Both mutex and semaphore must be deleted on rollback.
    assert_eq!(
        fixture::mutex_create_count() - mtx_created,
        fixture::mutex_delete_count() - mtx_deleted
    );
    assert_eq!(
        fixture::sem_create_count() - sem_created,
        fixture::sem_delete_count() - sem_deleted
    );
    assert_eq!(fixture::active_internal_task_count(), 0);

    fixture::set_fail_next_event_group_create(false);
    runtime::initialize().expect("second init should succeed");
    assert!(runtime::shutdown().is_ok());
}

// ---------------------------------------------------------------------------
// 7. Full shutdown/reinitialize cycle
// ---------------------------------------------------------------------------

#[test]
fn shutdown_releases_all_service_resources_and_reinitializes() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    let timer = FreeRtosTimer::new(
        "cyc",
        Duration::from_millis(10),
        TimerMode::OneShot,
        Box::new(move || {
            f.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();

    timer.start().unwrap();
    advance_ms(10);
    assert_eq!(fired.load(Ordering::Relaxed), 1);
    drop(timer);

    // Shut down — must succeed (no leaked timer handles).
    assert!(runtime::shutdown().is_ok());
    // Worker was created by start() and joined during shutdown.
    // After shutdown, the fixture's internal thread map is cleaned
    // up by fixture::reset() — verify reinit works.

    // Re-initialize and run a second timer.
    fixture::set_wait_mode(FixtureWaitMode::Virtual);
    runtime::initialize().unwrap();
    let fired2 = Arc::new(AtomicU32::new(0));
    let f2 = Arc::clone(&fired2);
    let t2 = FreeRtosTimer::new(
        "cyc2",
        Duration::from_millis(5),
        TimerMode::OneShot,
        Box::new(move || {
            f2.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();
    t2.start().unwrap();
    advance_ms(5);
    assert_eq!(fired2.load(Ordering::Relaxed), 1);
    drop(t2);
    assert!(runtime::shutdown().is_ok());
}
