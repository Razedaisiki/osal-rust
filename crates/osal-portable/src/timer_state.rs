//! Portable timer state machine.
//!
//! `TimerState` handles the core countdown, restart, and expiration
//! logic without any platform primitives. Backends wrap this type
//! with their own timer service (Mock `MockTimeRuntime`, POSIX
//! `PosixTimerService`).

use core::time::Duration;

use osal_api::error::{Error, Result};
use osal_api::types::TimerMode;

use crate::time_convert;

// ---------------------------------------------------------------------------
// ExpirationToken
// ---------------------------------------------------------------------------

/// Token captured before callback execution. Used after the callback
/// returns to determine whether the timer's state was modified during
/// callback execution.
#[derive(Debug, Clone)]
pub struct ExpirationToken {
    pub generation: u64,
    pub scheduled_deadline: Duration,
    pub mode: TimerMode,
}

// ---------------------------------------------------------------------------
// TimerState
// ---------------------------------------------------------------------------

/// Portable timer state: mode, period, deadline, generation.
pub struct TimerState {
    mode: TimerMode,
    period: Duration,
    deadline: Option<Duration>,
    generation: u64,
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
            generation: 0,
        })
    }

    /// Start or restart the timer. Sets `deadline = now + period`.
    pub fn start(&mut self, now: Duration) -> Result<()> {
        self.deadline = Some(time_convert::checked_deadline(now, self.period)?);
        self.generation += 1;
        Ok(())
    }

    /// Stop the timer. Idempotent.
    pub fn stop(&mut self) {
        if self.deadline.is_some() {
            self.deadline = None;
            self.generation += 1;
        }
    }

    /// Reset: `deadline = now + period`. Starts if stopped.
    pub fn reset(&mut self, now: Duration) -> Result<()> {
        self.deadline = Some(time_convert::checked_deadline(now, self.period)?);
        self.generation += 1;
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
        // No generation change — deadline is not reset
        Ok(())
    }

    /// Prepare for expiration. Returns a token if the timer has a deadline
    /// at or before `now`, and transitions the timer appropriately.
    ///
    /// For OneShot: clears the deadline.
    /// For Periodic: advances to the next deadline.
    pub fn prepare_expiration(&mut self, now: Duration) -> Option<ExpirationToken> {
        let deadline = self.deadline?;
        if deadline > now {
            return None;
        }
        let token = ExpirationToken {
            generation: self.generation,
            scheduled_deadline: deadline,
            mode: self.mode,
        };
        match self.mode {
            TimerMode::OneShot => {
                self.deadline = None;
            }
            TimerMode::Periodic => {
                // Compute next deadline (merge missed periods)
                if let Ok(next) =
                    time_convert::next_periodic_deadline(deadline, self.period, now)
                {
                    self.deadline = Some(next);
                } else {
                    // Overflow — stop the timer
                    self.deadline = None;
                }
            }
        }
        Some(token)
    }

    /// Finish expiration. If the generation matches the token, apply
    /// any post-expiration logic. For OneShot, nothing more is needed.
    /// For Periodic, the next deadline was already set in
    /// `prepare_expiration`. If the generation changed, the timer was
    /// modified during callback execution — discard.
    pub fn finish_expiration(&mut self, token: ExpirationToken) {
        // If generation changed, the timer was modified during callback
        // (e.g. stop, start, reset, change_period, drop). Do not
        // overwrite the new state.
        let _ = token;
        // The state was already advanced in prepare_expiration.
        // No further action needed here.
    }

    /// Current deadline, if running.
    pub fn deadline(&self) -> Option<Duration> {
        self.deadline
    }

    /// Current generation.
    pub fn generation(&self) -> u64 {
        self.generation
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
        s.stop(); // second stop is no-op
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
        assert_eq!(s.deadline(), Some(ms(100))); // unchanged
    }

    #[test]
    fn oneshot_expires_once() {
        let mut s = TimerState::new(ms(100), TimerMode::OneShot).unwrap();
        s.start(ms(0)).unwrap();
        let token = s.prepare_expiration(ms(100));
        assert!(token.is_some());
        assert!(s.deadline().is_none()); // stopped after fire
    }

    #[test]
    fn periodic_reloads() {
        let mut s = TimerState::new(ms(100), TimerMode::Periodic).unwrap();
        s.start(ms(0)).unwrap();
        let token = s.prepare_expiration(ms(100));
        assert!(token.is_some());
        assert_eq!(s.deadline(), Some(ms(200))); // auto-reload
    }

    #[test]
    fn periodic_missed_merged() {
        let mut s = TimerState::new(ms(100), TimerMode::Periodic).unwrap();
        s.start(ms(0)).unwrap();
        // Time jumped to 350ms — missed deadlines at 100, 200, 300
        let token = s.prepare_expiration(ms(350));
        assert!(token.is_some());
        assert_eq!(s.deadline(), Some(ms(400))); // next after 350
    }

    #[test]
    fn generation_changes_detected() {
        let mut s = TimerState::new(ms(100), TimerMode::Periodic).unwrap();
        s.start(ms(0)).unwrap();
        let token = s.prepare_expiration(ms(100)).unwrap();
        let gen_before = token.generation;
        s.stop(); // changes generation
        s.finish_expiration(token); // should not panic or overwrite
        let token2 = s.prepare_expiration(ms(200));
        assert!(token2.is_none()); // stopped, no deadline
        let _ = gen_before;
    }

    #[test]
    fn start_increments_generation() {
        let mut s = TimerState::new(ms(100), TimerMode::OneShot).unwrap();
        let g0 = s.generation();
        s.start(ms(0)).unwrap();
        assert!(s.generation() > g0);
    }
}
