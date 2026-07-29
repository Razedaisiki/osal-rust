//! FreeRTOS Timer lifecycle and callback concurrency tests.
//!
//! Tests that start the timer worker are grouped in this file to avoid
//! cross-test state interference (the worker thread must be fully shut
//! down between tests).

#![cfg(feature = "testkit")]

use core::time::Duration;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use osal_api::error::Error;
use osal_api::traits::timer::Timer;
use osal_api::types::TimerMode;
use osal_backend_freertos::runtime;
use osal_backend_freertos::timer::FreeRtosTimer;
use osal_backend_freertos_sys::fixture;

fn setup() {
    let _ = runtime::shutdown();
    fixture::reset();
    runtime::initialize().expect("initialize runtime");
}

fn teardown() {
    // Shutdown must succeed — all timers were dropped and the
    // worker should have processed all deregistrations.
    match runtime::shutdown() {
        Ok(()) | Err(Error::NotInitialized) => {}
        Err(_) => {
            thread::sleep(Duration::from_millis(20));
            assert!(
                runtime::shutdown().is_ok(),
                "runtime shutdown failed after timer lifecycle test"
            );
        }
    }
}

fn oneshot_tx(period_ms: u64, tx: mpsc::Sender<u64>, value: u64) -> FreeRtosTimer {
    FreeRtosTimer::new(
        "test",
        Duration::from_millis(period_ms),
        TimerMode::OneShot,
        Box::new(move || {
            tx.send(value).ok();
        }),
    )
    .expect("create timer")
}

// ---------------------------------------------------------------------------
// Comprehensive lifecycle test — all worker-dependent tests in one function
// since the worker must be cleanly shut down between test binaries.
// ---------------------------------------------------------------------------

#[test]
fn timer_lifecycle_and_callbacks() {
    setup();

    // --- OneShot fires once ---
    {
        let (tx, rx) = mpsc::channel();
        let timer = oneshot_tx(10, tx, 99);
        timer.start().unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_millis(500)).unwrap(), 99);
        // Should not fire again.
        thread::sleep(Duration::from_millis(50));
        assert!(rx.try_recv().is_err());
    }

    // --- Periodic fires multiple ---
    {
        let (tx, rx) = mpsc::channel();
        let count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count2 = Arc::clone(&count);
        let timer = FreeRtosTimer::new(
            "p",
            Duration::from_millis(10),
            TimerMode::Periodic,
            Box::new(move || {
                let n = count2.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if n >= 3 {
                    tx.send(n).ok();
                }
            }),
        )
        .unwrap();
        timer.start().unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(500)).unwrap() >= 3);
        timer.stop().unwrap();
    }

    // --- Callback can stop itself ---
    {
        let (tx, rx) = mpsc::channel();
        let timer_slot: Arc<std::sync::Mutex<Option<FreeRtosTimer>>> =
            Arc::new(std::sync::Mutex::new(None));
        let slot2 = Arc::clone(&timer_slot);
        let timer = FreeRtosTimer::new(
            "self-stop",
            Duration::from_millis(10),
            TimerMode::Periodic,
            Box::new(move || {
                tx.send(1).ok();
                if let Some(ref t) = *slot2.lock().unwrap() {
                    t.stop().unwrap();
                }
            }),
        )
        .unwrap();
        *timer_slot.lock().unwrap() = Some(timer.clone());
        timer.start().unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_millis(500)).unwrap(), 1);
        // Break the reference cycle: callback→Arc→Option<Timer>→RuntimeLease
        // would prevent the last Timer handle from ever dropping.
        timer_slot.lock().unwrap().take();
        thread::sleep(Duration::from_millis(50));
        assert!(rx.try_recv().is_err());
    }

    // --- Clone shares control ---
    {
        let (tx, rx) = mpsc::channel();
        let timer = oneshot_tx(200, tx.clone(), 42);
        let clone = timer.clone();
        timer.start().unwrap();
        clone.stop().unwrap();
        thread::sleep(Duration::from_millis(50));
        assert!(rx.try_recv().is_err());
    }

    // --- Last drop prevents callback ---
    {
        let (tx, rx) = mpsc::channel();
        let timer = oneshot_tx(500, tx, 1);
        timer.start().unwrap();
        drop(timer);
        thread::sleep(Duration::from_millis(50));
        assert!(rx.try_recv().is_err());
    }

    // --- Drop non-last clone keeps timer ---
    {
        let (tx, rx) = mpsc::channel();
        let timer = oneshot_tx(30, tx.clone(), 1);
        let clone = timer.clone();
        timer.start().unwrap();
        drop(clone);
        assert_eq!(rx.recv_timeout(Duration::from_millis(500)).unwrap(), 1);
    }

    teardown();
}
