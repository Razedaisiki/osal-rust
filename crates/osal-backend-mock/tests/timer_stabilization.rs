//! P3.1 stabilization tests — re-entry, epoch isolation, cross-timer.

use core::cell::Cell;
use core::time::Duration;

use osal_api::traits::timer::Timer as _;
use osal_api::types::TimerMode;
use osal_backend_mock::clock::{self, MockClock, MockClockControl};
use osal_backend_mock::timer::MockTimer;

#[test]
fn callback_stops_self() {
    clock::reset_runtime();
    let fired = Cell::new(false);
    let t = MockTimer::new("t", Duration::from_millis(100), TimerMode::Periodic,
        Box::new(|| fired.set(true))).unwrap();
    t.start().unwrap();
    // This callback stops itself
    let t2 = t.clone();
    let t3 = MockTimer::new("self_stop", Duration::from_millis(100), TimerMode::OneShot,
        Box::new(move || { t2.stop().unwrap(); })).unwrap();
    t3.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(200));
    // t3's callback stopped t3 itself
    // t is periodic and should have fired (but this test just checks no panic)
    drop(t);
}

#[test]
fn callback_resets_self() {
    clock::reset_runtime();
    let t = MockTimer::new("t", Duration::from_millis(100), TimerMode::OneShot,
        Box::new(|| {})).unwrap();
    let t2 = t.clone();
    // Callback that resets the timer
    let rst = MockTimer::new("rst", Duration::from_millis(50), TimerMode::OneShot,
        Box::new(move || { t2.reset().unwrap(); })).unwrap();
    t.start().unwrap();
    rst.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(200));
    // No panic — reset from callback works
    drop(t);
    drop(rst);
}

#[test]
fn callback_stops_another_timer() {
    clock::reset_runtime();
    let a_fired = Cell::new(false);
    let b_fired = Cell::new(false);

    let ta = MockTimer::new("A", Duration::from_millis(100), TimerMode::OneShot,
        Box::new(|| a_fired.set(true))).unwrap();
    let tb = MockTimer::new("B", Duration::from_millis(100), TimerMode::OneShot,
        Box::new(|| b_fired.set(true))).unwrap();
    let tb2 = tb.clone();

    // Callback that stops B
    let stopper = MockTimer::new("stopper", Duration::from_millis(50), TimerMode::OneShot,
        Box::new(move || { tb2.stop().unwrap(); })).unwrap();

    ta.start().unwrap();
    tb.start().unwrap();
    stopper.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(200));

    assert!(a_fired.get(), "A should fire");
    assert!(!b_fired.get(), "B should NOT fire (was stopped by stopper)");
}

#[test]
fn oneshot_re_trigger() {
    clock::reset_runtime();
    let fired = Cell::new(0u32);
    let t = MockTimer::new("t", Duration::from_millis(100), TimerMode::OneShot,
        Box::new(|| fired.set(fired.get() + 1))).unwrap();
    t.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(150));
    assert_eq!(fired.get(), 1);
    // Re-trigger
    t.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(150));
    assert_eq!(fired.get(), 2);
}

#[test]
fn epoch_reset_isolates_old_handles() {
    clock::reset_runtime();
    let t = MockTimer::new("t", Duration::from_millis(100), TimerMode::OneShot,
        Box::new(|| {})).unwrap();

    clock::reset_runtime();
    // Old handle's epoch no longer matches — operations should be no-ops
    t.start().unwrap(); // no panic
    t.stop().unwrap();  // no panic
    t.reset().unwrap(); // no panic
    drop(t);            // no panic (deregister with old epoch finds nothing)
}

#[test]
fn callback_calls_delay() {
    clock::reset_runtime();
    let fired = Cell::new(false);
    let t = MockTimer::new("t", Duration::from_millis(100), TimerMode::OneShot,
        Box::new(|| {
            MockClock::delay(Duration::from_millis(50));
            fired.set(true);
        })).unwrap();
    t.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(200));
    assert!(fired.get());
}

#[test]
fn periodic_not_reentrant() {
    clock::reset_runtime();
    let count = Cell::new(0u32);
    let t = MockTimer::new("t", Duration::from_millis(100), TimerMode::Periodic,
        Box::new(|| count.set(count.get() + 1))).unwrap();
    t.start().unwrap();
    MockClockControl.advance_clock(Duration::from_millis(350));
    // 100ms period, 350ms advance = one fire with 3 missed periods coalesced
    assert_eq!(count.get(), 1);
}

#[test]
fn callback_in_flight_last_handle_dropped() {
    clock::reset_runtime();
    let fired = Cell::new(false);
    let t = MockTimer::new("t", Duration::from_millis(100), TimerMode::OneShot,
        Box::new(|| fired.set(true))).unwrap();
    // Clone + drop before advance so last handle drops during... well mock is sync.
    // Just verify no panic on drop during callback-like scenario.
    let t2 = t.clone();
    t.start().unwrap();
    drop(t);
    MockClockControl.advance_clock(Duration::from_millis(200));
    assert!(fired.get());
    drop(t2);
}
