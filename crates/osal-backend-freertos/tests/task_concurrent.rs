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
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::thread;

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::task::{Task, TaskBuilder};
use osal_api::types::ExitCode;
use osal_backend_freertos::runtime;
use osal_backend_freertos::task::{FreeRtosTask, FreeRtosTaskBuilder};
use osal_backend_freertos_sys::fixture;

// Serialize task tests: they mutate shared static runtime/fixture state
// and must not run concurrently.
static TASK_TEST_LOCK: Mutex<()> = Mutex::new(());

struct TestGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl TestGuard {
    fn new() -> Self {
        let lock = TASK_TEST_LOCK.lock().expect("task test lock poisoned");
        let _ = runtime::shutdown();
        fixture::reset();
        runtime::initialize().expect("initialize");
        TestGuard { _lock: lock }
    }
}

impl Drop for TestGuard {
    fn drop(&mut self) {
        match runtime::shutdown() {
            Ok(()) | Err(Error::NotInitialized) => {}
            Err(e) => panic!("test leaked runtime lease or object: {e:?}"),
        }
        fixture::reset();
    }
}

// ---------------------------------------------------------------------------
// Join — NoWait / After(0) on running task
// ---------------------------------------------------------------------------

#[test]
fn join_no_wait_running_returns_timeout() {
    let _guard = TestGuard::new();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(()).ok();
                thread::sleep(Duration::from_millis(50));
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2))
            .expect("task started");

        assert_eq!(task.join(Timeout::NoWait), Err(Error::Timeout));
        task.join(Timeout::Forever).expect("join");
    }
}

#[test]
fn join_zero_running_returns_timeout() {
    let _guard = TestGuard::new();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(()).ok();
                thread::sleep(Duration::from_millis(50));
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2))
            .expect("task started");

        assert_eq!(
            task.join(Timeout::After(Duration::ZERO)),
            Err(Error::Timeout)
        );
        task.join(Timeout::Forever).expect("join");
    }
}

// ---------------------------------------------------------------------------
// Join — Forever
// ---------------------------------------------------------------------------

#[test]
fn forever_join_wakes_on_completion() {
    let _guard = TestGuard::new();
    {
        let task = FreeRtosTaskBuilder::new()
            .spawn(|| { /* immediate return */ })
            .expect("spawn");

        assert_eq!(task.join(Timeout::Forever), Ok(ExitCode::SUCCESS));
    }
}

// ---------------------------------------------------------------------------
// Join — finite timeout
// ---------------------------------------------------------------------------

#[test]
fn finite_join_times_out() {
    let _guard = TestGuard::new();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(()).ok();
                thread::sleep(Duration::from_millis(500));
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2))
            .expect("task started");

        assert_eq!(
            task.join(Timeout::After(Duration::from_millis(10))),
            Err(Error::Timeout)
        );
        task.join(Timeout::Forever).expect("join");
    }
}

#[test]
fn finite_timeout_can_retry_forever() {
    let _guard = TestGuard::new();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(()).ok();
                thread::sleep(Duration::from_millis(200));
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2))
            .expect("task started");

        assert_eq!(
            task.join(Timeout::After(Duration::from_millis(10))),
            Err(Error::Timeout)
        );
        assert_eq!(task.join(Timeout::Forever), Ok(ExitCode::SUCCESS));
    }
}

// ---------------------------------------------------------------------------
// Repeated join returns cached result
// ---------------------------------------------------------------------------

#[test]
fn repeated_join_returns_cached_result() {
    let _guard = TestGuard::new();
    {
        let task = FreeRtosTaskBuilder::new()
            .spawn(|| { /* immediate */ })
            .expect("spawn");

        assert_eq!(task.join(Timeout::Forever), Ok(ExitCode::SUCCESS));
        assert_eq!(task.join(Timeout::NoWait), Ok(ExitCode::SUCCESS));
        assert_eq!(task.join(Timeout::Forever), Ok(ExitCode::SUCCESS));
        assert_eq!(
            task.join(Timeout::After(Duration::ZERO)),
            Ok(ExitCode::SUCCESS)
        );
    }
}

// ---------------------------------------------------------------------------
// Concurrent joiners
// ---------------------------------------------------------------------------

#[test]
fn two_joiners_receive_same_result() {
    let _guard = TestGuard::new();
    {
        let task = FreeRtosTaskBuilder::new()
            .spawn(|| { /* immediate */ })
            .expect("spawn");

        let t1 = task.clone();
        let t2 = task.clone();
        let barrier = Arc::new(Barrier::new(3));
        let b1 = Arc::clone(&barrier);
        let b2 = Arc::clone(&barrier);

        let (tx, rx) = mpsc::channel();
        let tx1 = tx.clone();
        let tx2 = tx.clone();
        drop(tx);

        let h1 = thread::spawn(move || {
            b1.wait();
            tx1.send(t1.join(Timeout::Forever)).ok();
        });
        let h2 = thread::spawn(move || {
            b2.wait();
            tx2.send(t2.join(Timeout::Forever)).ok();
        });

        barrier.wait();

        let r1 = rx.recv_timeout(Duration::from_secs(2)).expect("joiner 1");
        let r2 = rx.recv_timeout(Duration::from_secs(2)).expect("joiner 2");
        assert_eq!(r1, Ok(ExitCode::SUCCESS));
        assert_eq!(r2, Ok(ExitCode::SUCCESS));

        h1.join().expect("h1");
        h2.join().expect("h2");
    }
    thread::sleep(Duration::from_millis(20));
}

#[test]
fn late_joiner_receives_cached_result() {
    let _guard = TestGuard::new();
    {
        let task = FreeRtosTaskBuilder::new()
            .spawn(|| { /* immediate */ })
            .expect("spawn");

        // First joiner completes.
        task.join(Timeout::Forever).expect("first join");

        // Late joiner (after task finished) gets cached result immediately.
        let t2 = task.clone();
        let handle = thread::spawn(move || t2.join(Timeout::NoWait));
        let result = handle.join().expect("late joiner panicked");
        assert_eq!(result, Ok(ExitCode::SUCCESS));
    }
    thread::sleep(Duration::from_millis(20));
}

// ---------------------------------------------------------------------------
// Self-join
// ---------------------------------------------------------------------------

#[test]
fn self_join_returns_busy() {
    let _guard = TestGuard::new();
    {
        let slot: Arc<Mutex<Option<FreeRtosTask>>> = Arc::new(Mutex::new(None));
        let (result_tx, result_rx) = mpsc::channel();

        let task = {
            let slot = Arc::clone(&slot);
            FreeRtosTaskBuilder::new()
                .spawn(move || {
                    // Wait for the main thread to give us our own handle.
                    let own = loop {
                        if let Some(t) = slot.lock().unwrap().clone() {
                            break t;
                        }
                        thread::yield_now();
                    };
                    // Self-join must return Busy.
                    result_tx.send(own.join(Timeout::Forever)).unwrap();
                })
                .expect("spawn")
        };

        // Give the task its own handle.
        *slot.lock().unwrap() = Some(task.clone());

        let self_join_result = result_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("self-join did not complete");
        assert_eq!(self_join_result, Err(Error::Busy));

        // Main thread can still join the task.
        task.join(Timeout::Forever).expect("join");
    }
    thread::sleep(Duration::from_millis(20));
}

// ---------------------------------------------------------------------------
// Drop does not cancel
// ---------------------------------------------------------------------------

#[test]
fn drop_handle_does_not_cancel_task() {
    let _guard = TestGuard::new();
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
}

// ---------------------------------------------------------------------------
// Finished join ignores scheduler state
// ---------------------------------------------------------------------------

#[test]
fn finished_join_works_when_scheduler_not_started() {
    let _guard = TestGuard::new();
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
}

// ---------------------------------------------------------------------------
// Scheduler-state preconditions for blocking join
// ---------------------------------------------------------------------------

#[test]
fn blocking_join_not_started_returns_not_initialized() {
    let _guard = TestGuard::new();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(()).ok();
                thread::sleep(Duration::from_millis(100));
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2))
            .expect("task started");

        fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::NotStarted);

        assert_eq!(task.join(Timeout::Forever), Err(Error::NotInitialized));

        fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Running);
    }
    // The task may still be running — wait for it.
    thread::sleep(Duration::from_millis(200));
}

// ---------------------------------------------------------------------------
// Shutdown lifecycle
// ---------------------------------------------------------------------------

#[test]
fn shutdown_busy_while_task_running() {
    let _guard = TestGuard::new();
    {
        let (tx, rx) = mpsc::channel();
        let task = FreeRtosTaskBuilder::new()
            .spawn(move || {
                tx.send(()).ok();
                thread::sleep(Duration::from_millis(100));
            })
            .expect("spawn");
        rx.recv_timeout(Duration::from_secs(2))
            .expect("task started");

        assert_eq!(runtime::shutdown(), Err(Error::Busy));

        drop(task);
    }
    // Allow the task trampoline thread to fully exit before guard Drop
    // attempts runtime shutdown.
    thread::sleep(Duration::from_millis(200));
}

#[test]
fn shutdown_busy_while_finished_handle_alive() {
    let _guard = TestGuard::new();
    {
        let task = FreeRtosTaskBuilder::new()
            .spawn(|| { /* immediate */ })
            .expect("spawn");
        task.join(Timeout::Forever).expect("join");

        // Task is finished but handle is alive — RuntimeLease is held.
        assert_eq!(runtime::shutdown(), Err(Error::Busy));
    }
    // Allow the task trampoline thread to fully exit and release its Arcs.
    thread::sleep(Duration::from_millis(50));
}

#[test]
fn shutdown_succeeds_after_last_handle_drop() {
    let _guard = TestGuard::new();
    {
        let task = FreeRtosTaskBuilder::new()
            .spawn(|| { /* immediate */ })
            .expect("spawn");
        task.join(Timeout::Forever).expect("join");
        drop(task);
    }
    // Wait for trampoline thread to fully exit and release its
    // RuntimeLease before guard Drop calls shutdown.
    thread::sleep(Duration::from_millis(50));
}

// ---------------------------------------------------------------------------
// Stack bytes → words verification
// ---------------------------------------------------------------------------

#[test]
fn stack_bytes_rounds_up_to_words() {
    let _guard = TestGuard::new();
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
}

#[test]
fn stack_clamps_to_minimum_native_depth() {
    let _guard = TestGuard::new();
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
}

// ---------------------------------------------------------------------------
// Priority mapping
// ---------------------------------------------------------------------------

#[test]
fn priority_reports_requested_value() {
    let _guard = TestGuard::new();
    {
        let task = FreeRtosTaskBuilder::new()
            .priority(7)
            .spawn(|| {})
            .expect("spawn");
        assert_eq!(task.priority(), 7);
        task.join(Timeout::Forever).expect("join");
        assert_eq!(task.priority(), 7);
    }
}

#[test]
fn native_priority_saturates() {
    let _guard = TestGuard::new();
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
}

// ---------------------------------------------------------------------------
// Constructor failure — zero stack
// ---------------------------------------------------------------------------

#[test]
fn zero_stack_rejected() {
    let _guard = TestGuard::new();
    {
        let result = FreeRtosTaskBuilder::new().stack_size(0).spawn(|| {});
        assert!(matches!(result, Err(Error::InvalidParameter)));
    }
}

// ---------------------------------------------------------------------------
// Stress cycle
// ---------------------------------------------------------------------------

#[test]
fn task_stress_50_cycles() {
    let _guard = TestGuard::new();
    {
        for i in 0..50 {
            let task = match FreeRtosTaskBuilder::new().spawn(move || {
                let _ = i;
            }) {
                Ok(t) => t,
                Err(e) => panic!("spawn {i}: {e:?}"),
            };
            assert_eq!(task.join(Timeout::Forever), Ok(ExitCode::SUCCESS));
        }
    }
}
