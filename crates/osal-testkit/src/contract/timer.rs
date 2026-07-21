//! Contract tests for the `Timer` trait.

use alloc::boxed::Box;
use core::time::Duration;

use osal_api::error::Error;
use osal_api::traits::timer::Timer as _;
use osal_api::types::TimerMode;

use crate::factory::{ControlledTimerFactory, TimerFactory};

// ===========================================================================
// Core contracts (all backends)
// ===========================================================================

pub fn zero_period_rejected<F: TimerFactory>(factory: &F) {
    let result = factory.create_timer(
        "zero",
        Duration::ZERO,
        TimerMode::OneShot,
        factory.dummy_callback(),
    );
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

pub fn created_stopped<F: TimerFactory>(factory: &F) {
    let _t = factory
        .create_timer(
            "t",
            Duration::from_millis(100),
            TimerMode::OneShot,
            factory.dummy_callback(),
        )
        .unwrap();
}

pub fn stop_idempotent<F: TimerFactory>(factory: &F) {
    let t = factory
        .create_timer(
            "t",
            Duration::from_millis(100),
            TimerMode::OneShot,
            factory.dummy_callback(),
        )
        .unwrap();
    t.stop().unwrap();
    t.stop().unwrap();
}

pub fn change_period_zero_rejected<F: TimerFactory>(factory: &F) {
    let t = factory
        .create_timer(
            "t",
            Duration::from_millis(100),
            TimerMode::OneShot,
            factory.dummy_callback(),
        )
        .unwrap();
    assert!(matches!(
        t.change_period(Duration::ZERO),
        Err(Error::InvalidParameter)
    ));
}

pub fn clone_shares_control<F: TimerFactory>(factory: &F)
where
    F::Timer: Clone,
{
    let t1 = factory
        .create_timer(
            "t",
            Duration::from_millis(100),
            TimerMode::OneShot,
            factory.dummy_callback(),
        )
        .unwrap();
    let t2 = t1.clone();
    t2.stop().unwrap();
}

pub fn drop_clone_preserves<F: TimerFactory>(factory: &F)
where
    F::Timer: Clone,
{
    let t1 = factory
        .create_timer(
            "t",
            Duration::from_millis(100),
            TimerMode::OneShot,
            factory.dummy_callback(),
        )
        .unwrap();
    let t2 = t1.clone();
    drop(t1);
    t2.stop().unwrap();
}

pub fn run_core_contracts<F: TimerFactory>(factory: &F)
where
    F::Timer: Clone,
{
    zero_period_rejected::<F>(factory);
    created_stopped::<F>(factory);
    stop_idempotent::<F>(factory);
    change_period_zero_rejected::<F>(factory);
    clone_shares_control::<F>(factory);
    drop_clone_preserves::<F>(factory);
}

// ===========================================================================
// Controlled contracts (Mock only)
// ===========================================================================

pub fn oneshot_fires_once<F: ControlledTimerFactory>(factory: &F) {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicU32, Ordering};
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);
    let t = factory
        .create_timer(
            "os",
            Duration::from_millis(100),
            TimerMode::OneShot,
            Box::new(move || {
                f.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .unwrap();
    t.start().unwrap();
    factory.advance_clock(Duration::from_millis(99));
    assert_eq!(fired.load(Ordering::Relaxed), 0);
    factory.advance_clock(Duration::from_millis(1));
    assert_eq!(fired.load(Ordering::Relaxed), 1);
    factory.advance_clock(Duration::from_millis(200));
    assert_eq!(fired.load(Ordering::Relaxed), 1);
}

pub fn periodic_fires_multiple<F: ControlledTimerFactory>(factory: &F) {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicU32, Ordering};
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);
    let t = factory
        .create_timer(
            "p",
            Duration::from_millis(100),
            TimerMode::Periodic,
            Box::new(move || {
                f.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .unwrap();
    t.start().unwrap();
    // Advance in steps so each period is processed separately
    factory.advance_clock(Duration::from_millis(100));
    assert_eq!(fired.load(Ordering::Relaxed), 1);
    factory.advance_clock(Duration::from_millis(100));
    assert_eq!(fired.load(Ordering::Relaxed), 2);
    factory.advance_clock(Duration::from_millis(100));
    assert!(fired.load(Ordering::Relaxed) >= 3);
}

pub fn stop_prevents_callback<F: ControlledTimerFactory>(factory: &F) {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    let fired = Arc::new(AtomicBool::new(false));
    let f = Arc::clone(&fired);
    let t = factory
        .create_timer(
            "s",
            Duration::from_millis(100),
            TimerMode::OneShot,
            Box::new(move || {
                f.store(true, Ordering::Relaxed);
            }),
        )
        .unwrap();
    t.start().unwrap();
    t.stop().unwrap();
    factory.advance_clock(Duration::from_millis(200));
    assert!(!fired.load(Ordering::Relaxed));
}

pub fn reset_restarts_deadline<F: ControlledTimerFactory>(factory: &F) {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicU32, Ordering};
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);
    let t = factory
        .create_timer(
            "r",
            Duration::from_millis(100),
            TimerMode::OneShot,
            Box::new(move || {
                f.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .unwrap();
    t.start().unwrap();
    factory.advance_clock(Duration::from_millis(50));
    t.reset().unwrap();
    factory.advance_clock(Duration::from_millis(60));
    assert_eq!(fired.load(Ordering::Relaxed), 0);
    factory.advance_clock(Duration::from_millis(50));
    assert_eq!(fired.load(Ordering::Relaxed), 1);
}

pub fn missed_expiration_coalesced<F: ControlledTimerFactory>(factory: &F) {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicU32, Ordering};
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);
    let t = factory
        .create_timer(
            "m",
            Duration::from_millis(100),
            TimerMode::Periodic,
            Box::new(move || {
                f.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .unwrap();
    t.start().unwrap();
    factory.advance_clock(Duration::from_millis(350));
    assert_eq!(fired.load(Ordering::Relaxed), 1);
}

pub fn run_controlled_contracts<F: ControlledTimerFactory>(factory: &F) {
    oneshot_fires_once::<F>(factory);
    periodic_fires_multiple::<F>(factory);
    stop_prevents_callback::<F>(factory);
    reset_restarts_deadline::<F>(factory);
    missed_expiration_coalesced::<F>(factory);
}

// ===========================================================================
// Realtime contracts (POSIX only)
// ===========================================================================

#[cfg(feature = "std")]
#[cfg(test)]
pub mod realtime {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Instant;

    pub fn oneshot_fires_within_bounds<F: TimerFactory>(factory: &F) {
        let fired = Arc::new(AtomicU32::new(0));
        let f = fired.clone();
        let t = factory
            .create_timer(
                "rt",
                Duration::from_millis(50),
                TimerMode::OneShot,
                Box::new(move || {
                    f.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();
        let start = Instant::now();
        t.start().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fired.load(Ordering::SeqCst), 1);
        assert!(start.elapsed() >= Duration::from_millis(40));
    }

    pub fn periodic_fires_multiple<F: TimerFactory>(factory: &F) {
        let fired = Arc::new(AtomicU32::new(0));
        let f = fired.clone();
        let t = factory
            .create_timer(
                "rp",
                Duration::from_millis(30),
                TimerMode::Periodic,
                Box::new(move || {
                    f.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();
        t.start().unwrap();
        std::thread::sleep(Duration::from_millis(120));
        t.stop().unwrap();
        assert!(fired.load(Ordering::SeqCst) >= 2);
    }

    pub fn stop_prevents_future_callbacks<F: TimerFactory>(factory: &F) {
        let fired = Arc::new(AtomicU32::new(0));
        let f = fired.clone();
        let t = factory
            .create_timer(
                "rs",
                Duration::from_millis(50),
                TimerMode::Periodic,
                Box::new(move || {
                    f.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();
        t.start().unwrap();
        std::thread::sleep(Duration::from_millis(80));
        t.stop().unwrap();
        let count = fired.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fired.load(Ordering::SeqCst), count);
    }

    pub fn reset_delays_callback<F: TimerFactory>(factory: &F) {
        let fired = Arc::new(AtomicU32::new(0));
        let f = fired.clone();
        let t = factory
            .create_timer(
                "rr",
                Duration::from_millis(200),
                TimerMode::OneShot,
                Box::new(move || {
                    f.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();
        t.start().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        t.reset().unwrap();
        let start = Instant::now();
        while fired.load(Ordering::SeqCst) == 0 {
            assert!(
                start.elapsed() < Duration::from_secs(1),
                "timed out waiting for callback"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(start.elapsed() >= Duration::from_millis(150));
    }

    pub fn run_realtime_contracts<F: TimerFactory>(factory: &F) {
        oneshot_fires_within_bounds::<F>(factory);
        periodic_fires_multiple::<F>(factory);
        stop_prevents_future_callbacks::<F>(factory);
        reset_delays_callback::<F>(factory);
    }
}
