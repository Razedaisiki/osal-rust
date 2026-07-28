//! Task concurrency and boundary tests for the FreeRTOS backend.
//!
//! Covers join variants, self-join, concurrent joiners, scheduler-state
//! preconditions, lifecycle (drop, shutdown), and stack/priority mapping.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit task_concurrent -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use core::time::Duration;
use std::sync::mpsc;
use std::thread;

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::task::{Task, TaskBuilder};
use osal_api::types::ExitCode;
use osal_backend_freertos::runtime;
use osal_backend_freertos::task::FreeRtosTaskBuilder;
use osal_backend_freertos_sys::fixture;

fn setup() {
    fixture::reset();
    let _ = runtime::shutdown();
    runtime::initialize().expect("initialize");
}

fn teardown() {
    match runtime::shutdown() {
        Ok(()) | Err(Error::NotInitialized) => {}
        Err(e) => panic!("test leaked runtime lease or object: {e:?}"),
    }
    fixture::reset();
}

// ---------------------------------------------------------------------------
// Join — NoWait / After(0) on running task
// ---------------------------------------------------------------------------

#[test]
fn join_no_wait_running_returns_timeout() {
    setup();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(()).ok();
                thread::sleep(Duration::from_millis(50));
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2)).expect("task started");

        assert_eq!(task.join(Timeout::NoWait), Err(Error::Timeout));
        task.join(Timeout::Forever).expect("join");
    }
    teardown();
}

#[test]
fn join_zero_running_returns_timeout() {
    setup();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(()).ok();
                thread::sleep(Duration::from_millis(50));
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2)).expect("task started");

        assert_eq!(
            task.join(Timeout::After(Duration::ZERO)),
            Err(Error::Timeout)
        );
        task.join(Timeout::Forever).expect("join");
    }
    teardown();
}

// ---------------------------------------------------------------------------
// Join — Forever
// ---------------------------------------------------------------------------

#[test]
fn forever_join_wakes_on_completion() {
    setup();
    {
        let task = FreeRtosTaskBuilder::new()
            .spawn(|| { /* immediate return */ })
            .expect("spawn");

        assert_eq!(task.join(Timeout::Forever), Ok(ExitCode::SUCCESS));
    }
    teardown();
}

// ---------------------------------------------------------------------------
// Join — finite timeout
// ---------------------------------------------------------------------------

#[test]
fn finite_join_times_out() {
    setup();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(()).ok();
                thread::sleep(Duration::from_millis(500));
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2)).expect("task started");

        assert_eq!(
            task.join(Timeout::After(Duration::from_millis(10))),
            Err(Error::Timeout)
        );
        task.join(Timeout::Forever).expect("join");
    }
    teardown();
}

#[test]
fn finite_timeout_can_retry_forever() {
    setup();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(()).ok();
                thread::sleep(Duration::from_millis(200));
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2)).expect("task started");

        assert_eq!(
            task.join(Timeout::After(Duration::from_millis(10))),
            Err(Error::Timeout)
        );
        assert_eq!(task.join(Timeout::Forever), Ok(ExitCode::SUCCESS));
    }
    teardown();
}

// ---------------------------------------------------------------------------
// Repeated join returns cached result
// ---------------------------------------------------------------------------

#[test]
fn repeated_join_returns_cached_result() {
    setup();
    {
        let task = FreeRtosTaskBuilder::new()
            .spawn(|| { /* immediate */ })
            .expect("spawn");

        assert_eq!(task.join(Timeout::Forever), Ok(ExitCode::SUCCESS));
        assert_eq!(task.join(Timeout::NoWait), Ok(ExitCode::SUCCESS));
        assert_eq!(task.join(Timeout::Forever), Ok(ExitCode::SUCCESS));
        assert_eq!(task.join(Timeout::After(Duration::ZERO)), Ok(ExitCode::SUCCESS));
    }
    teardown();
}

// ---------------------------------------------------------------------------
// Self-join
// ---------------------------------------------------------------------------

#[test]
fn self_join_returns_busy() {
    setup();
    {
        let (tx, rx) = mpsc::channel();
        let txc = tx.clone();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                txc.send(()).ok();
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2)).expect("task started");
        task.join(Timeout::Forever).expect("join");
    }
    thread::sleep(Duration::from_millis(20));
    teardown();
}

// ---------------------------------------------------------------------------
// Drop does not cancel
// ---------------------------------------------------------------------------

#[test]
fn drop_handle_does_not_cancel_task() {
    setup();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(42).ok();
            })
            .expect("spawn");

        drop(task);

        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("task should complete despite dropped handle");
        assert_eq!(result, 42);
    }
    // Give the task trampoline time to drop its Arc and release the lease.
    thread::sleep(Duration::from_millis(20));
    teardown();
}

// ---------------------------------------------------------------------------
// Finished join ignores scheduler state
// ---------------------------------------------------------------------------

#[test]
fn finished_join_works_when_scheduler_not_started() {
    setup();
    {
        let task = FreeRtosTaskBuilder::new()
            .spawn(|| { /* immediate */ })
            .expect("spawn");
        task.join(Timeout::Forever).expect("join");

        fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::NotStarted);

        // Already-finished task should join without the scheduler.
        assert_eq!(task.join(Timeout::Forever), Ok(ExitCode::SUCCESS));
        assert_eq!(task.join(Timeout::NoWait), Ok(ExitCode::SUCCESS));

        fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Running);
    }
    // Give the task trampoline time to fully exit.
    thread::sleep(Duration::from_millis(20));
    teardown();
}

// ---------------------------------------------------------------------------
// Scheduler-state preconditions for blocking join
// ---------------------------------------------------------------------------

#[test]
fn blocking_join_not_started_returns_not_initialized() {
    setup();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(()).ok();
                thread::sleep(Duration::from_millis(100));
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2)).expect("task started");

        fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::NotStarted);

        assert_eq!(
            task.join(Timeout::Forever),
            Err(Error::NotInitialized)
        );

        fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Running);
    }
    // The task may still be running — wait for it.
    thread::sleep(Duration::from_millis(200));
    teardown();
}

// ---------------------------------------------------------------------------
// Shutdown lifecycle
// ---------------------------------------------------------------------------

#[test]
fn shutdown_busy_while_task_running() {
    setup();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(()).ok();
                thread::sleep(Duration::from_millis(100));
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2)).expect("task started");

        assert_eq!(runtime::shutdown(), Err(Error::Busy));

        drop(task);
    }
    thread::sleep(Duration::from_millis(200));
    assert!(runtime::shutdown().is_ok());
    fixture::reset();
}

#[test]
fn shutdown_busy_while_finished_handle_alive() {
    setup();
    {
        let task = FreeRtosTaskBuilder::new()
            .spawn(|| { /* immediate */ })
            .expect("spawn");
        task.join(Timeout::Forever).expect("join");

        // Task is finished but handle is alive — RuntimeLease is held.
        assert_eq!(runtime::shutdown(), Err(Error::Busy));
    }
    assert!(runtime::shutdown().is_ok());
    fixture::reset();
}

#[test]
fn shutdown_succeeds_after_last_handle_drop() {
    setup();
    {
        let task = FreeRtosTaskBuilder::new()
            .spawn(|| { /* immediate */ })
            .expect("spawn");
        task.join(Timeout::Forever).expect("join");
        drop(task);
    }
    assert!(runtime::shutdown().is_ok());
    fixture::reset();
}

// ---------------------------------------------------------------------------
// Stack bytes → words verification
// ---------------------------------------------------------------------------

#[test]
fn stack_bytes_rounds_up_to_words() {
    setup();
    {
        let caps = runtime::capabilities_for_test().expect("capabilities");
        let word_size = caps.stack_word_size as usize;

        let task = FreeRtosTaskBuilder::new()
            .stack_size(1) // 1 byte → 1 word
            .spawn(|| {})
            .expect("spawn");
        task.join(Timeout::Forever).expect("join");
        drop(task);

        // Verify that the stack depth was at least the minimum.
        let stack_words = fixture::last_stack_depth_words() as usize;
        assert!(
            stack_words >= caps.minimal_stack_depth_words as usize,
            "stack_words={stack_words} should be >= minimal={}",
            caps.minimal_stack_depth_words
        );
        assert!(
            stack_words >= 1,
            "stack_words={stack_words} should be >= 1 byte worth ({word_size} byte words)"
        );
    }
    teardown();
}

#[test]
fn stack_clamps_to_minimum_native_depth() {
    setup();
    {
        let task = FreeRtosTaskBuilder::new()
            .stack_size(1)
            .spawn(|| {})
            .expect("spawn");
        task.join(Timeout::Forever).expect("join");
        drop(task);

        let stack_words = fixture::last_stack_depth_words();
        let caps = runtime::capabilities_for_test().expect("capabilities");
        assert!(
            stack_words >= caps.minimal_stack_depth_words,
            "stack_words={stack_words} should be >= minimal={}",
            caps.minimal_stack_depth_words
        );
    }
    teardown();
}

// ---------------------------------------------------------------------------
// Priority mapping
// ---------------------------------------------------------------------------

#[test]
fn priority_reports_requested_value() {
    setup();
    {
        let task = FreeRtosTaskBuilder::new()
            .priority(7)
            .spawn(|| {})
            .expect("spawn");
        assert_eq!(task.priority(), 7);
        task.join(Timeout::Forever).expect("join");
        assert_eq!(task.priority(), 7);
    }
    teardown();
}

#[test]
fn native_priority_saturates() {
    setup();
    {
        let task = FreeRtosTaskBuilder::new()
            .priority(100) // above configMAX_PRIORITIES=8
            .spawn(|| {})
            .expect("spawn");
        task.join(Timeout::Forever).expect("join");
        drop(task);

        let native_prio = fixture::last_native_priority();
        let caps = runtime::capabilities_for_test().expect("capabilities");
        assert!(
            native_prio < caps.max_priorities,
            "native_prio={native_prio} should be < max_priorities={}",
            caps.max_priorities
        );
    }
    teardown();
}

// ---------------------------------------------------------------------------
// Constructor failure — zero stack
// ---------------------------------------------------------------------------

#[test]
fn zero_stack_rejected() {
    setup();
    {
        let result = FreeRtosTaskBuilder::new()
            .stack_size(0)
            .spawn(|| {});
        assert!(matches!(result, Err(Error::InvalidParameter)));
    }
    teardown();
}

// ---------------------------------------------------------------------------
// Stress cycle
// ---------------------------------------------------------------------------

#[test]
fn task_stress_50_cycles() {
    setup();
    {
        for i in 0..50 {
            let task = match FreeRtosTaskBuilder::new()
                .spawn(move || { let _ = i; })
            {
                Ok(t) => t,
                Err(e) => panic!("spawn {i}: {e:?}"),
            };
            assert_eq!(task.join(Timeout::Forever), Ok(ExitCode::SUCCESS));
        }
    }
    teardown();
}
