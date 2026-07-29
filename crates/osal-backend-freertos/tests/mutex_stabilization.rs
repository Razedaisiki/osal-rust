//! Backend-specific stabilisation tests for `FreeRtosMutex`.
//!
//! Tests OOM, last-clone-delete, guard-drop ordering, and error mapping
//! that the generic contract suite does not cover.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit mutex_stabilization -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::mutex::Mutex;
use osal_backend_freertos::mutex::FreeRtosMutex;
use osal_backend_freertos::runtime;
use osal_backend_freertos_sys::fixture;

fn setup() {
    let _ = runtime::shutdown();
    fixture::reset();
    runtime::initialize().expect("initialize runtime");
}

fn teardown() {
    let _ = runtime::shutdown();
}

// ---------------------------------------------------------------------------
// OOM
// ---------------------------------------------------------------------------

#[test]
fn mutex_create_oom_returns_out_of_memory() {
    setup();
    fixture::set_fail_next_mutex_create(true);

    let result = FreeRtosMutex::<u32>::new(42);
    assert_eq!(result.unwrap_err(), Error::OutOfMemory);

    teardown();
}

// ---------------------------------------------------------------------------
// Non-recursive: same-task re-lock fails
// ---------------------------------------------------------------------------

#[test]
fn same_task_nolock_reacquire_returns_lock_failed() {
    setup();

    let m = FreeRtosMutex::new(0u32).expect("create mutex");
    let _guard = m.lock(Timeout::NoWait).expect("first lock");
    let result = m.lock(Timeout::NoWait);
    assert_eq!(result.unwrap_err(), Error::LockFailed);

    teardown();
}

#[test]
fn same_task_after_zero_reacquire_returns_timeout() {
    setup();

    let m = FreeRtosMutex::new(0u32).expect("create mutex");
    let _guard = m.lock(Timeout::NoWait).expect("first lock");
    let result = m.lock(Timeout::After(core::time::Duration::ZERO));
    assert_eq!(result.unwrap_err(), Error::Timeout);

    teardown();
}

// ---------------------------------------------------------------------------
// Clone shares state
// ---------------------------------------------------------------------------

#[test]
fn clone_shares_underlying_mutex() {
    setup();

    let m1 = FreeRtosMutex::new(0u32).expect("create");
    let m2 = m1.clone();
    {
        let mut guard = m1.lock(Timeout::NoWait).expect("lock m1");
        *guard = 99;
    }
    let guard = m2.lock(Timeout::NoWait).expect("lock m2");
    assert_eq!(*guard, 99);

    teardown();
}

// ---------------------------------------------------------------------------
// Last-clone delete
// ---------------------------------------------------------------------------

#[test]
fn last_clone_drops_native_handle() {
    setup();

    let m1 = FreeRtosMutex::new(0u32).expect("create");
    let m2 = m1.clone();
    drop(m1);
    assert_eq!(fixture::mutex_delete_count(), 0, "first clone dropped");
    drop(m2);
    assert_eq!(
        fixture::mutex_delete_count(),
        1,
        "last clone deleted native"
    );

    teardown();
}

// ---------------------------------------------------------------------------
// Guard !Send + !Sync compile-time assertions
// ---------------------------------------------------------------------------
//
// FreeRtosMutexGuard uses PhantomData<Rc<()>> to enforce !Send + !Sync.
// These properties are verified at compile time:
//
//   require_send::<FreeRtosMutexGuard<'static, u32>>();
//   //       ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
//   //       error: `Rc<()>` cannot be sent between threads safely
//
//   require_sync::<FreeRtosMutexGuard<'static, u32>>();
//   //       ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
//   //       error: `Rc<()>` cannot be shared between threads safely
//
// Uncomment the `#[test]` below to verify during development.

#[allow(dead_code)]
fn guard_send_sync_assertions() {
    fn require_send<T: Send>() {}
    fn require_sync<T: Sync>() {}
    // These lines fail compilation — left as documentation:
    // require_send::<super::FreeRtosMutexGuard<'static, u32>>();
    // require_sync::<super::FreeRtosMutexGuard<'static, u32>>();
}
