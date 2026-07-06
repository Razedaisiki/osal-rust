//! Portable timer state machine — pre-advance model.
//!
//! When a timer expires, its state is advanced **before** the callback
//! executes (OneShot clears deadline, Periodic advances to next).
//! Callbacks may freely call `start`, `stop`, `reset`, `change_period`
//! — these operations directly overwrite the pre-advanced state.
//! No post-callback correction is needed.

use core::time::Duration;

use osal_api::error::{Error, Result};
use osal_api::types::TimerMode;

use crate::time_convert;

/// Portable timer state: mode, period, deadline.
pub struct TimerState {
    mode: TimerMode,
    period: Duration,
    deadline: Option<Duration>,
}

impl TimerState {
    /// Create a new stopped timer.
    ///
    /// Returns `Error::InvalidParameter` if `period` is zero.
    pub fn new(period: Duration, mode: TimerMode) -> Result<Self> {
        if period == Duration::ZERO {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            mode,
            period,
            deadline: None,
        })
    }

    /// Start or restart the timer. Sets `deadline = now + period`.
    pub fn start(&mut self, now: Duration) -> Result<()> {
        self.deadline = Some(time_convert::checked_deadline(now, self.period)?);
        Ok(())
    }

    /// Stop the timer. Idempotent.
    pub fn stop(&mut self) {
        self.deadline = None;
    }

    /// Reset: `deadline = now + period`. Starts if stopped.
    pub fn reset(&mut self, now: Duration) -> Result<()> {
        self.deadline = Some(time_convert::checked_deadline(now, self.period)?);
        Ok(())
    }

    /// Change the period. Does not change the current deadline.
    ///
    /// Returns `Error::InvalidParameter` if `new_period` is zero.
    pub fn change_period(&mut self, new_period: Duration) -> Result<()> {
        if new_period == Duration::ZERO {
            return Err(Error::InvalidParameter);
        }
        self.period = new_period;
        Ok(())
    }

    /// Advance state on expiry. Returns `true` if the timer expired
    /// (deadline was set and `deadline <= now`), and transitions the
    /// state before returning (pre-advance model).
    ///
    /// - **OneShot**: deadline = None (stopped after firing).
    /// - **Periodic**: advances to next periodic deadline, merging
    ///   missed periods. If overflow, stops.
    pub fn advance_on_expiry(&mut self, now: Duration) -> bool {
        let deadline = match self.deadline {
            Some(d) if d <= now => d,
            _ => return false,
        };
        match self.mode {
            TimerMode::OneShot => {
                self.deadline = None;
            }
            TimerMode::Periodic => {
                if let Ok(next) = time_convert::next_periodic_deadline(deadline, self.period, now) {
                    self.deadline = Some(next);
                } else {
                    self.deadline = None; // overflow
                }
            }
        }
        true
    }

    /// Current deadline, if running.
    pub fn deadline(&self) -> Option<Duration> {
        self.deadline
    }

    /// Current period.
    pub fn period(&self) -> Duration {
        self.period
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn reject_zero_period() {
        assert_eq!(
            TimerState::new(Duration::ZERO, TimerMode::OneShot).unwrap_err(),
            Error::InvalidParameter
        );
    }

    #[test]
    fn created_stopped() {
        let s = TimerState::new(ms(100), TimerMode::OneShot).unwrap();
        assert!(s.deadline().is_none());
    }

    #[test]
    fn start_sets_deadline() {
        let mut s = TimerState::new(ms(100), TimerMode::OneShot).unwrap();
        s.start(ms(0)).unwrap();
        assert_eq!(s.deadline(), Some(ms(100)));
    }

    #[test]
    fn start_running_is_reset() {
        let mut s = TimerState::new(ms(100), TimerMode::OneShot).unwrap();
        s.start(ms(0)).unwrap();
        s.start(ms(50)).unwrap();
        assert_eq!(s.deadline(), Some(ms(150)));
    }

    #[test]
    fn stop_idempotent() {
        let mut s = TimerState::new(ms(100), TimerMode::OneShot).unwrap();
        s.start(ms(0)).unwrap();
        s.stop();
        s.stop();
        assert!(s.deadline().is_none());
    }

    #[test]
    fn reset_stopped_starts() {
        let mut s = TimerState::new(ms(100), TimerMode::OneShot).unwrap();
        s.reset(ms(50)).unwrap();
        assert_eq!(s.deadline(), Some(ms(150)));
    }

    #[test]
    fn change_period_rejects_zero() {
        let mut s = TimerState::new(ms(100), TimerMode::OneShot).unwrap();
        assert_eq!(
            s.change_period(Duration::ZERO).unwrap_err(),
            Error::InvalidParameter
        );
    }

    #[test]
    fn change_period_does_not_reset_deadline() {
        let mut s = TimerState::new(ms(100), TimerMode::OneShot).unwrap();
        s.start(ms(0)).unwrap();
        s.change_period(ms(500)).unwrap();
        assert_eq!(s.deadline(), Some(ms(100)));
    }

    #[test]
    fn oneshot_expires_once() {
        let mut s = TimerState::new(ms(100), TimerMode::OneShot).unwrap();
        s.start(ms(0)).unwrap();
        assert!(s.advance_on_expiry(ms(100)));
        assert!(s.deadline().is_none());
    }

    #[test]
    fn periodic_reloads() {
        let mut s = TimerState::new(ms(100), TimerMode::Periodic).unwrap();
        s.start(ms(0)).unwrap();
        assert!(s.advance_on_expiry(ms(100)));
        assert_eq!(s.deadline(), Some(ms(200)));
    }

    #[test]
    fn periodic_missed_merged() {
        let mut s = TimerState::new(ms(100), TimerMode::Periodic).unwrap();
        s.start(ms(0)).unwrap();
        assert!(s.advance_on_expiry(ms(350)));
        assert_eq!(s.deadline(), Some(ms(400)));
    }

    #[test]
    fn not_expired() {
        let mut s = TimerState::new(ms(100), TimerMode::OneShot).unwrap();
        s.start(ms(0)).unwrap();
        assert!(!s.advance_on_expiry(ms(50)));
        assert_eq!(s.deadline(), Some(ms(100)));
    }
}
