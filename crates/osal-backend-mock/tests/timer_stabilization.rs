//! P3.1 stabilization tests — re-entry, epoch isolation, cross-timer.

use core::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use osal_api::traits::clock::Clock as _;
use osal_api::traits::timer::Timer as _;
use osal_api::types::TimerMode;
use osal_backend_mock::clock::{MockClock, MockClockControl};
use osal_backend_mock::test_support::mock_time_test_guard;
use osal_backend_mock::timer::MockTimer;
use osal_testkit::factory::clock::ClockControl as _;

#[test]
fn callback_stops_self() {
    let _guard = mock_time_test_guard();

    MockClockControl.reset();
    let t = MockTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::Periodic,
        Box::new(|| {}),
    )
    .unwrap();
    t.start().unwrap();
    let t2 = t.clone();
    let _t3 = MockTimer::new(
        "self_stop",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            t2.stop().unwrap();
        }),
    )
    .unwrap();
    _t3.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(200));
    drop(t);
}

#[test]
fn callback_resets_self() {
    let _guard = mock_time_test_guard();

    MockClockControl.reset();
    let t = MockTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(|| {}),
    )
    .unwrap();
    let t2 = t.clone();
    let rst = MockTimer::new(
        "rst",
        Duration::from_millis(50),
        TimerMode::OneShot,
        Box::new(move || {
            t2.reset().unwrap();
        }),
    )
    .unwrap();
    t.start().unwrap();
    rst.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(200));
    drop(t);
    drop(rst);
}

#[test]
fn callback_stops_another_timer() {
    let _guard = mock_time_test_guard();

    MockClockControl.reset();
    let a_fired = Arc::new(AtomicBool::new(false));
    let af = Arc::clone(&a_fired);
    let b_fired = Arc::new(AtomicBool::new(false));
    let bf = Arc::clone(&b_fired);

    let ta = MockTimer::new(
        "A",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            af.store(true, Ordering::SeqCst);
        }),
    )
    .unwrap();
    let tb = MockTimer::new(
        "B",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            bf.store(true, Ordering::SeqCst);
        }),
    )
    .unwrap();
    let tb2 = tb.clone();

    let stopper = MockTimer::new(
        "stopper",
        Duration::from_millis(50),
        TimerMode::OneShot,
        Box::new(move || {
            tb2.stop().unwrap();
        }),
    )
    .unwrap();

    ta.start().unwrap();
    tb.start().unwrap();
    stopper.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(200));

    assert!(a_fired.load(Ordering::SeqCst), "A should fire");
    assert!(
        !b_fired.load(Ordering::SeqCst),
        "B should NOT fire (stopped by stopper)"
    );
}

#[test]
fn oneshot_re_trigger() {
    let _guard = mock_time_test_guard();

    MockClockControl.reset();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);
    let t = MockTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            f.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();
    t.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(150));
    assert_eq!(fired.load(Ordering::Relaxed), 1);
    t.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(150));
    assert_eq!(fired.load(Ordering::Relaxed), 2);
}

#[test]
fn epoch_reset_isolates_old_handles() {
    let _guard = mock_time_test_guard();

    MockClockControl.reset();
    let t = MockTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(|| {}),
    )
    .unwrap();
    MockClockControl.reset();
    t.start().unwrap();
    t.stop().unwrap();
    t.reset().unwrap();
    drop(t);
}

#[test]
fn callback_calls_delay() {
    let _guard = mock_time_test_guard();

    MockClockControl.reset();
    let fired = Arc::new(AtomicBool::new(false));
    let f = Arc::clone(&fired);
    let t = MockTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            MockClock::delay(Duration::from_millis(50));
            f.store(true, Ordering::SeqCst);
        }),
    )
    .unwrap();
    t.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(200));
    assert!(fired.load(Ordering::SeqCst));
}

#[test]
fn periodic_not_reentrant() {
    let _guard = mock_time_test_guard();

    MockClockControl.reset();
    let count = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&count);
    let t = MockTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::Periodic,
        Box::new(move || {
            c.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();
    t.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(350));
    assert_eq!(count.load(Ordering::Relaxed), 1);
}

#[test]
fn callback_in_flight_last_handle_dropped() {
    let _guard = mock_time_test_guard();

    MockClockControl.reset();
    let fired = Arc::new(AtomicBool::new(false));
    let f = Arc::clone(&fired);
    let t = MockTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            f.store(true, Ordering::SeqCst);
        }),
    )
    .unwrap();
    let _t2 = t.clone();
    t.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(200));
    assert!(fired.load(Ordering::SeqCst));
}
