# ADR 0009: Clock Time Domain Model

## Status

Accepted (2026-07-06)

## Context

OSAL's Queue, Mutex, and Semaphore all use `Timeout::After(Duration)`
for timed operations. The Clock trait provides the underlying time
source. P3 must decide:

1. Whether Clock is a static trait or an instantiated object.
2. How time is managed in the Mock backend.
3. What clock source POSIX uses.
4. Whether deadline arithmetic uses checked operations.

## Decision

1. **Clock is a static trait.** It has no `&self` methods — all functions
   are associated functions on a zero-sized type. This reflects the fact
   that each backend has exactly one system monotonic clock.

2. **One time domain per backend:**
   - POSIX: `clock_gettime(CLOCK_MONOTONIC)`
   - Mock: `MockTimeRuntime` virtual `Duration` counter
   - FreeRTOS (future): system tick counter

3. **`elapsed()` has a saturating default:**
   ```rust
   fn elapsed(since: Duration) -> Duration {
       Self::now().saturating_sub(since)
   }
   ```
   Backends only need to implement `now()` and `delay()`.

4. **Deadline arithmetic uses checked operations.** Overflow in
   `now + timeout` must be detected, not silently wrapped. Portable
   helpers in `osal-portable/src/time_convert.rs` provide this.

5. **`delay(ZERO)` must return immediately.** All backends.

6. **Mock `reset_clock()` is test-only.** It resets the virtual time to
   zero and clears the timer registry. It is not part of the public
   `Clock` trait — only `MockClockControl` exposes it.

7. **Time never goes backward.** `now()` must be monotonic. Mock time
   only advances via `advance_clock()` or `delay()`.

## Rationale

- Static trait avoids Clock handles proliferating through every API that
  needs time. Queue, Mutex, Semaphore, and Timer all use the same
  system-wide monotonic clock.
- One domain per backend ensures consistency: a `Timeout::After(100ms)`
  means the same thing regardless of which primitive is being waited on.
- `saturating_sub` for `elapsed` prevents panics on clock source
  glitches (though monotonic clocks should never go backward).
- Checked deadline arithmetic prevents silent wraparound on 32-bit
  platforms with large durations.

## Consequences

- `MockClock` no longer uses a bare `AtomicU64` — it reads from a shared
  `MockTimeRuntime` that also manages the timer registry.
- `MockClock::delay(d)` advances the virtual clock AND dispatches any
  timers that expire during the advance.
- `abs_deadline` helper moves from `sys/condvar.rs` to `sys/time.rs`.
- All existing timeout code (Queue, Mutex, Semaphore) continues to work
  with the same `condvar::abs_deadline` function.
