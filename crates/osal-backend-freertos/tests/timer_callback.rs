//! FreeRTOS Timer callback reentry and lock-free destruction tests.
//!
//! All tests use deterministic Virtual mode.

#![cfg(feature = "testkit")]

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::time::Duration;
use std::sync::{Arc, Mutex};

use osal_api::error::Error;
use osal_api::traits::timer::Timer;
use osal_api::types::TimerMode;
use osal_backend_freertos::runtime;
use osal_backend_freertos::timer::{FreeRtosTimer, flush_timer_service};
use osal_backend_freertos_sys::fixture;
use osal_backend_freertos_sys::fixture::FixtureWaitMode;

// ---------------------------------------------------------------------------
// TestGuard
// ---------------------------------------------------------------------------

struct TestGuard;

impl TestGuard {
    fn new() -> Self {
        let _ = runtime::shutdown();
        fixture::reset();
        fixture::set_wait_mode(FixtureWaitMode::Virtual);
        runtime::initialize().expect("initialize");
        TestGuard
    }
}

impl Drop for TestGuard {
    fn drop(&mut self) {
        flush_timer_service();
        match runtime::shutdown() {
            Ok(()) | Err(Error::NotInitialized) => {}
            Err(_) => {
                std::thread::sleep(Duration::from_millis(20));
                flush_timer_service();
                assert!(runtime::shutdown().is_ok(), "runtime shutdown failed");
            }
        }
        fixture::set_wait_mode(FixtureWaitMode::Realtime);
    }
}

fn advance_ms(ms: u64) {
    fixture::advance_ticks(ms);
    flush_timer_service();
}

// ---------------------------------------------------------------------------
// 1. Callback can stop itself
// ---------------------------------------------------------------------------

#[test]
fn callback_can_stop_itself() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);
    let slot: Arc<Mutex<Option<FreeRtosTimer>>> = Arc::new(Mutex::new(None));
    let s = Arc::clone(&slot);

    let timer = FreeRtosTimer::new(
        "self-stop",
        Duration::from_millis(10),
        TimerMode::Periodic,
        Box::new(move || {
            f.fetch_add(1, Ordering::Relaxed);
            if let Some(ref t) = *s.lock().unwrap() {
                t.stop().unwrap();
            }
        }),
    )
    .unwrap();

    *slot.lock().unwrap() = Some(timer.clone());
    timer.start().unwrap();
    advance_ms(15); // past first deadline

    assert_eq!(fired.load(Ordering::Relaxed), 1);
    // Break reference cycle.
    slot.lock().unwrap().take();
    advance_ms(100); // well past
    assert_eq!(fired.load(Ordering::Relaxed), 1, "should not fire again");
}

// ---------------------------------------------------------------------------
// 2. Callback can reset itself (OneShot → fires again)
// ---------------------------------------------------------------------------

#[test]
fn oneshot_callback_can_reset_itself() {
    let _guard = TestGuard::new();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let slot: Arc<Mutex<Option<FreeRtosTimer>>> = Arc::new(Mutex::new(None));
    let s = Arc::clone(&slot);

    let timer = FreeRtosTimer::new(
        "self-reset",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            let n = c.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 1 {
                if let Some(ref t) = *s.lock().unwrap() {
                    t.reset().unwrap();
                }
            }
        }),
    )
    .unwrap();

    *slot.lock().unwrap() = Some(timer.clone());
    timer.start().unwrap();
    advance_ms(100); // first fire, self-reset
    assert_eq!(counter.load(Ordering::Relaxed), 1);

    advance_ms(99); // 99 ms after reset (=199 from start)
    assert_eq!(counter.load(Ordering::Relaxed), 1);

    slot.lock().unwrap().take();
    advance_ms(1); // 200 from start = 100 after reset
    assert_eq!(counter.load(Ordering::Relaxed), 2);
}

// ---------------------------------------------------------------------------
// 3. OneShot callback can restart itself
// ---------------------------------------------------------------------------

#[test]
fn oneshot_callback_can_restart_itself() {
    let _guard = TestGuard::new();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let slot: Arc<Mutex<Option<FreeRtosTimer>>> = Arc::new(Mutex::new(None));
    let s = Arc::clone(&slot);

    let timer = FreeRtosTimer::new(
        "self-restart",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            let n = c.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 1 {
                if let Some(ref t) = *s.lock().unwrap() {
                    t.start().unwrap(); // stopped → start
                }
            }
        }),
    )
    .unwrap();

    *slot.lock().unwrap() = Some(timer.clone());
    timer.start().unwrap();
    advance_ms(100);
    assert_eq!(counter.load(Ordering::Relaxed), 1);

    advance_ms(99);
    assert_eq!(counter.load(Ordering::Relaxed), 1);

    slot.lock().unwrap().take();
    advance_ms(1);
    assert_eq!(counter.load(Ordering::Relaxed), 2);
}

// ---------------------------------------------------------------------------
// 4. Callback can change its own period
// ---------------------------------------------------------------------------

#[test]
fn callback_can_change_own_period() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);
    let slot: Arc<Mutex<Option<FreeRtosTimer>>> = Arc::new(Mutex::new(None));
    let s = Arc::clone(&slot);

    let timer = FreeRtosTimer::new(
        "self-cp",
        Duration::from_millis(100),
        TimerMode::Periodic,
        Box::new(move || {
            let n = f.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 1 {
                if let Some(ref t) = *s.lock().unwrap() {
                    t.change_period(Duration::from_millis(200)).unwrap();
                }
            }
        }),
    )
    .unwrap();

    *slot.lock().unwrap() = Some(timer.clone());
    timer.start().unwrap();

    // First fire at 100ms.  Pre-advance sets deadline=200 (using old
    // period 100).  Callback changes period to 200.
    advance_ms(100);
    assert_eq!(fired.load(Ordering::Relaxed), 1);

    // The pre-advanced deadline is 200, so second fire at 200 ms.
    advance_ms(99); // total 199
    assert_eq!(fired.load(Ordering::Relaxed), 1);

    slot.lock().unwrap().take();
    advance_ms(1); // total 200
    assert_eq!(fired.load(Ordering::Relaxed), 2);

    // Third fire uses NEW period: 200 + 200 = 400.
    advance_ms(199); // total 399
    assert_eq!(fired.load(Ordering::Relaxed), 2);

    advance_ms(1); // total 400
    assert_eq!(fired.load(Ordering::Relaxed), 3);
}

// ---------------------------------------------------------------------------
// 5. Callback can control another timer
// ---------------------------------------------------------------------------

#[test]
fn callback_can_control_another_timer() {
    let _guard = TestGuard::new();
    let a_fired = Arc::new(AtomicU32::new(0));
    let b_fired = Arc::new(AtomicU32::new(0));
    let af = Arc::clone(&a_fired);
    let bf = Arc::clone(&b_fired);
    let other_slot: Arc<Mutex<Option<FreeRtosTimer>>> = Arc::new(Mutex::new(None));
    let os = Arc::clone(&other_slot);

    // Timer A: starts timer B when it fires.
    let timer_a = FreeRtosTimer::new(
        "a",
        Duration::from_millis(50),
        TimerMode::OneShot,
        Box::new(move || {
            af.fetch_add(1, Ordering::Relaxed);
            if let Some(ref t) = *os.lock().unwrap() {
                t.start().unwrap();
            }
        }),
    )
    .unwrap();

    // Timer B: stopped at creation, started by A's callback.
    let timer_b = FreeRtosTimer::new(
        "b",
        Duration::from_millis(30),
        TimerMode::OneShot,
        Box::new(move || {
            bf.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();

    *other_slot.lock().unwrap() = Some(timer_b.clone());
    timer_a.start().unwrap();

    advance_ms(50); // A fires, starts B
    assert_eq!(a_fired.load(Ordering::Relaxed), 1);
    assert_eq!(
        b_fired.load(Ordering::Relaxed),
        0,
        "B started at 50, fires at 80"
    );

    advance_ms(29); // total 79
    assert_eq!(b_fired.load(Ordering::Relaxed), 0);

    other_slot.lock().unwrap().take();
    advance_ms(1); // total 80
    assert_eq!(b_fired.load(Ordering::Relaxed), 1);
}

// ---------------------------------------------------------------------------
// 6. Callback capture is dropped outside registry lock
// ---------------------------------------------------------------------------

struct DropProbe {
    dropped: Arc<AtomicBool>,
    other: Option<FreeRtosTimer>,
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Relaxed);
        // If we're inside the registry lock, operating on another
        // timer (which acquires the registry lock) would deadlock.
        // The test verifies this by making the drop path use the
        // other timer: stop() must succeed without deadlocking.
        if let Some(ref t) = self.other {
            t.stop().unwrap();
        }
    }
}

#[test]
fn callback_capture_dropped_outside_registry_lock() {
    let _guard = TestGuard::new();
    let dropped_flag = Arc::new(AtomicBool::new(false));

    // Create a sacrificial timer just so the probe has something to
    // call stop() on during drop.  The probe OWNS this timer clone.
    let other_timer = FreeRtosTimer::new(
        "other",
        Duration::from_millis(500),
        TimerMode::OneShot,
        Box::new(|| {}),
    )
    .unwrap();

    let probe = DropProbe {
        dropped: Arc::clone(&dropped_flag),
        other: Some(other_timer),
    };

    // `probe` is moved into the callback.  When the timer is dropped
    // after firing, `deregister` drops the callback, which drops
    // `probe` — this must happen OUTSIDE the registry lock.
    let timer = FreeRtosTimer::new(
        "capture",
        Duration::from_millis(10),
        TimerMode::OneShot,
        Box::new(move || {
            // `probe` is captured by value (moved into closure).
            // We just need to touch it so the compiler doesn't
            // optimize it away before the closure is dropped.
            let _ = &probe;
        }),
    )
    .unwrap();

    timer.start().unwrap();
    advance_ms(10);
    // Callback has fired; timer is auto-stopped.

    // Drop the last public handle.  deregister takes the callback
    // (containing probe) and drops it.  DropProbe::drop calls
    // other.stop() which acquires the registry lock.  If the callback
    // were dropped INSIDE the registry lock, this would deadlock.
    drop(timer);
    flush_timer_service();

    assert!(
        dropped_flag.load(Ordering::Relaxed),
        "DropProbe was not dropped"
    );
    // If we reached here without deadlocking, the callback capture
    // was correctly dropped outside the registry lock.
}
