//! Checked conversion between a tick-based kernel clock and [`Duration`].
//!
//! Tick-based RTOS kernels (FreeRTOS, ThreadX, Zephyr, etc.) increment a
//! counter on each tick interrupt.  The counter has a finite width and
//! wraps around.  This module provides:
//!
//! - Expansion of a coherent `{overflow_count, tick_count}` snapshot into
//!   a `u128` total-tick value.
//! - Conversion from total ticks to [`Duration`] (with saturation at
//!   `Duration::MAX` when the elapsed time would overflow the type).
//! - Ceiling conversion from [`Duration`] to ticks (so a non-zero
//!   Duration never maps to 0 ticks).
//! - The maximum finite tick count for a given `TickType_t` width
//!   (reserving the all-ones value as `portMAX_DELAY`-style sentinel).
//!
//! All arithmetic is checked — overflow produces
//! [`Error::Overflow`]; invalid parameters produce
//! [`Error::InvalidParameter`].

use core::time::Duration;

use osal_api::error::{Error, Result};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Configuration for a tick-based clock.
///
/// All backends that use a tick counter share this configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickConfig {
    /// Tick interrupt frequency in Hz (e.g. 1000 for a 1 ms tick).
    pub rate_hz: u32,
    /// Width of the native tick-counter type in bits: 16, 32, or 64.
    pub bits: u8,
}

/// A coherent snapshot of the kernel's tick state.
///
/// Captured atomically by the backend (e.g. via `vTaskSetTimeOutState`
/// on FreeRTOS).  The caller must guarantee that `overflow_count` and
/// `tick_count` were read from the same coherent sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickSnapshot {
    /// Number of times the native tick counter has overflowed.
    pub overflow_count: u64,
    /// Current native tick-counter value (`0 .. (1 << TickConfig::bits)`).
    pub tick_count: u64,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const NANOS_PER_SEC: u128 = 1_000_000_000;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Convert a tick snapshot to a [`Duration`].
///
/// Saturates to [`Duration::MAX`] when the elapsed time would overflow
/// the `Duration` type.  This preserves the monotonicity contract of
/// `Clock::now()` without requiring a fallible return type.
///
/// # Errors
///
/// - `Error::InvalidParameter` if `config.rate_hz` is 0 or `config.bits`
///   is not 16, 32, or 64.
pub fn snapshot_to_duration(snapshot: TickSnapshot, config: TickConfig) -> Result<Duration> {
    validate_config(&config)?;

    let total_ticks = expand_ticks(snapshot, config.bits);
    ticks_to_duration(total_ticks, config.rate_hz)
}

/// Convert a [`Duration`] to ticks, rounding **up** (ceiling).
///
/// A non-zero `duration` always maps to at least 1 tick.  `Duration::ZERO`
/// maps to 0 ticks.
///
/// # Errors
///
/// - `Error::InvalidParameter` if `tick_rate_hz` is 0.
/// - `Error::Overflow` if the intermediate arithmetic overflows.
pub fn duration_to_ticks_ceil(duration: Duration, tick_rate_hz: u32) -> Result<u128> {
    if tick_rate_hz == 0 {
        return Err(Error::InvalidParameter);
    }
    if duration.is_zero() {
        return Ok(0);
    }

    let nanos: u128 = duration.as_nanos();
    let rate: u128 = tick_rate_hz as u128;

    // ceil(nanos * rate / 1_000_000_000)
    // = (nanos * rate + 1_000_000_000 - 1) / 1_000_000_000
    let numerator = nanos
        .checked_mul(rate)
        .and_then(|v| v.checked_add(NANOS_PER_SEC - 1))
        .ok_or(Error::Overflow)?;

    Ok(numerator / NANOS_PER_SEC)
}

/// Return the maximum finite tick count for a given `TickType_t` width.
///
/// Reserves the all-ones sentinel value (e.g. FreeRTOS `portMAX_DELAY`),
/// returning `(1 << bits) - 2`.  For 64-bit ticks the sentinel is
/// `u64::MAX` and this returns `u64::MAX - 1`.
///
/// # Errors
///
/// - `Error::InvalidParameter` if `tick_bits` is not 16, 32, or 64.
pub fn max_finite_ticks(tick_bits: u8) -> Result<u64> {
    match tick_bits {
        16 => Ok((1u64 << 16) - 2),
        32 => Ok((1u64 << 32) - 2),
        64 => Ok(u64::MAX - 1),
        _ => Err(Error::InvalidParameter),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Validate [`TickConfig`] parameters.
fn validate_config(config: &TickConfig) -> Result<()> {
    if config.rate_hz == 0 {
        return Err(Error::InvalidParameter);
    }
    if !matches!(config.bits, 16 | 32 | 64) {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}

/// Expand a tick snapshot to a `u128` total-tick count.
///
/// `total = (overflow_count << bits) | tick_count`
fn expand_ticks(snapshot: TickSnapshot, tick_bits: u8) -> u128 {
    ((snapshot.overflow_count as u128) << tick_bits) | (snapshot.tick_count as u128)
}

/// Convert total ticks to [`Duration`], saturating at `Duration::MAX`.
fn ticks_to_duration(total_ticks: u128, rate_hz: u32) -> Result<Duration> {
    let rate: u128 = rate_hz as u128;

    let seconds = total_ticks / rate;
    let remainder = total_ticks % rate;
    let nanos = remainder * NANOS_PER_SEC / rate;

    // Saturate seconds at Duration::MAX's seconds component.
    // Duration::MAX has seconds = u64::MAX and nanos = 999_999_999.
    let secs_u64 = match u64::try_from(seconds) {
        Ok(s) => s,
        Err(_) => return Ok(Duration::MAX),
    };

    let nanos_u32 = nanos as u32;

    // We have valid secs and nanos.  Construct directly.
    Ok(Duration::new(secs_u64, nanos_u32))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- snapshot_to_duration -------------------------------------------------

    #[test]
    fn snapshot_1000hz_1000ticks_is_1_second() {
        let snap = TickSnapshot {
            overflow_count: 0,
            tick_count: 1000,
        };
        let cfg = TickConfig {
            rate_hz: 1000,
            bits: 32,
        };
        assert_eq!(snapshot_to_duration(snap, cfg).unwrap(), Duration::from_secs(1));
    }

    #[test]
    fn snapshot_100hz_1tick_is_10ms() {
        let snap = TickSnapshot {
            overflow_count: 0,
            tick_count: 1,
        };
        let cfg = TickConfig {
            rate_hz: 100,
            bits: 32,
        };
        assert_eq!(
            snapshot_to_duration(snap, cfg).unwrap(),
            Duration::from_millis(10)
        );
    }

    #[test]
    fn snapshot_128hz_nondivisible_truncates_to_ns() {
        let snap = TickSnapshot {
            overflow_count: 0,
            tick_count: 1,
        };
        let cfg = TickConfig {
            rate_hz: 128,
            bits: 32,
        };
        // 1 tick at 128 Hz = 1_000_000_000 / 128 = 7_812_500 ns
        assert_eq!(
            snapshot_to_duration(snap, cfg).unwrap(),
            Duration::from_nanos(7_812_500)
        );
    }

    #[test]
    fn snapshot_zero_is_zero() {
        let snap = TickSnapshot {
            overflow_count: 0,
            tick_count: 0,
        };
        let cfg = TickConfig {
            rate_hz: 1000,
            bits: 32,
        };
        assert_eq!(snapshot_to_duration(snap, cfg).unwrap(), Duration::ZERO);
    }

    #[test]
    fn snapshot_16bit_wrap_not_backwards() {
        // After a 16-bit wrap, now() must not decrease.
        let before = TickSnapshot {
            overflow_count: 0,
            tick_count: 65535,
        };
        let after = TickSnapshot {
            overflow_count: 1,
            tick_count: 0,
        };
        let cfg = TickConfig {
            rate_hz: 1000,
            bits: 16,
        };
        let d1 = snapshot_to_duration(before, cfg).unwrap();
        let d2 = snapshot_to_duration(after, cfg).unwrap();
        assert!(d2 > d1);
    }

    #[test]
    fn snapshot_32bit_wrap_not_backwards() {
        let before = TickSnapshot {
            overflow_count: 0,
            tick_count: u32::MAX as u64,
        };
        let after = TickSnapshot {
            overflow_count: 1,
            tick_count: 0,
        };
        let cfg = TickConfig {
            rate_hz: 1000,
            bits: 32,
        };
        let d1 = snapshot_to_duration(before, cfg).unwrap();
        let d2 = snapshot_to_duration(after, cfg).unwrap();
        assert!(d2 > d1);
    }

    #[test]
    fn snapshot_64bit_tick() {
        // 64-bit ticks: overflow_count is still meaningful.
        let snap = TickSnapshot {
            overflow_count: 1,
            tick_count: 1024,
        };
        let cfg = TickConfig {
            rate_hz: 1024,
            bits: 64,
        };
        // total = (1 << 64) + 1024 ticks at 1024 Hz
        // seconds = ((1<<64) + 1024) / 1024
        let total: u128 = (1u128 << 64) + 1024;
        let expected_secs = total / 1024;
        let expected_nanos = ((total % 1024) * 1_000_000_000 / 1024) as u32;
        let result = snapshot_to_duration(snap, cfg).unwrap();
        assert_eq!(result.as_secs(), expected_secs as u64);
        assert_eq!(result.subsec_nanos(), expected_nanos);
    }

    #[test]
    fn snapshot_duration_overflow_saturates() {
        // Enormous overflow count forces duration beyond Duration::MAX.
        let snap = TickSnapshot {
            overflow_count: u64::MAX,
            tick_count: u64::MAX,
        };
        let cfg = TickConfig {
            rate_hz: 1,
            bits: 64,
        };
        assert_eq!(snapshot_to_duration(snap, cfg).unwrap(), Duration::MAX);
    }

    #[test]
    fn snapshot_rate_zero_is_invalid() {
        let snap = TickSnapshot {
            overflow_count: 0,
            tick_count: 1,
        };
        let cfg = TickConfig {
            rate_hz: 0,
            bits: 32,
        };
        assert_eq!(
            snapshot_to_duration(snap, cfg).unwrap_err(),
            Error::InvalidParameter
        );
    }

    #[test]
    fn snapshot_bits_invalid() {
        let snap = TickSnapshot {
            overflow_count: 0,
            tick_count: 1,
        };
        for bad in [0u8, 8u8, 15u8, 17u8, 31u8, 33u8, 63u8, 65u8, 128u8] {
            let cfg = TickConfig {
                rate_hz: 1000,
                bits: bad,
            };
            assert_eq!(
                snapshot_to_duration(snap, cfg).unwrap_err(),
                Error::InvalidParameter,
                "bits={bad} should be rejected"
            );
        }
    }

    // -- duration_to_ticks_ceil -----------------------------------------------

    #[test]
    fn ceil_exact_tick_boundary() {
        // 10 ms at 100 Hz = exactly 1 tick.
        let d = Duration::from_millis(10);
        assert_eq!(duration_to_ticks_ceil(d, 100).unwrap(), 1);
    }

    #[test]
    fn ceil_1ns_rounds_up_to_1_tick() {
        // Even 1 ns at 1 Hz must produce at least 1 tick.
        assert_eq!(duration_to_ticks_ceil(Duration::from_nanos(1), 1).unwrap(), 1);
    }

    #[test]
    fn ceil_exact_tick_at_high_rate() {
        // 1 ms at 1000 Hz = exactly 1 tick.
        assert_eq!(
            duration_to_ticks_ceil(Duration::from_millis(1), 1000).unwrap(),
            1
        );
    }

    #[test]
    fn ceil_just_over_tick_boundary() {
        // 11 ms at 100 Hz = 1.1 ticks → ceil = 2.
        let d = Duration::from_millis(11);
        assert_eq!(duration_to_ticks_ceil(d, 100).unwrap(), 2);
    }

    #[test]
    fn ceil_zero_duration_is_zero_ticks() {
        assert_eq!(duration_to_ticks_ceil(Duration::ZERO, 1000).unwrap(), 0);
    }

    #[test]
    fn ceil_large_duration_checked() {
        // A Duration of nearly u64::MAX seconds should not overflow.
        let d = Duration::new(u64::MAX - 1, 999_999_999);
        let rate = 100_000u32;
        let result = duration_to_ticks_ceil(d, rate);
        assert!(result.is_ok());
        let ticks = result.unwrap();
        // Expected: ceil(d_ns * rate / 1e9).  Verify ticks > 0.
        assert!(ticks > 0);
    }

    #[test]
    fn ceil_max_values_does_not_overflow() {
        // Even Duration::MAX at u32::MAX rate fits in u128.
        // The intermediate u128 has enough headroom for this.
        let rate = u32::MAX;
        let result = duration_to_ticks_ceil(Duration::MAX, rate);
        assert!(result.is_ok());
        let ticks = result.unwrap();
        assert!(ticks > 0);
    }

    #[test]
    fn ceil_rate_zero_is_invalid() {
        assert_eq!(
            duration_to_ticks_ceil(Duration::from_secs(1), 0).unwrap_err(),
            Error::InvalidParameter
        );
    }

    // -- max_finite_ticks -----------------------------------------------------

    #[test]
    fn max_finite_16bit() {
        assert_eq!(max_finite_ticks(16).unwrap(), 65534);
    }

    #[test]
    fn max_finite_32bit() {
        assert_eq!(max_finite_ticks(32).unwrap(), 4_294_967_294);
    }

    #[test]
    fn max_finite_64bit() {
        assert_eq!(max_finite_ticks(64).unwrap(), u64::MAX - 1);
    }

    #[test]
    fn max_finite_invalid_bits() {
        for bad in [0u8, 8u8, 15u8, 17u8, 31u8, 33u8, 63u8, 65u8, 128u8] {
            assert_eq!(
                max_finite_ticks(bad).unwrap_err(),
                Error::InvalidParameter,
                "bits={bad} should be rejected"
            );
        }
    }
}
