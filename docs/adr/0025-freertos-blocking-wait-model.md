# ADR 0025: FreeRTOS Blocking Wait Model

## Status

Accepted (2026-07-26)

## Context

The OSAL Mutex and Semaphore traits expose four timeout modes:
`NoWait`, `After(Duration::ZERO)`, `After(d > 0)`, and `Forever`.
FreeRTOS provides `xSemaphoreTake(handle, ticks)` where `ticks` is a
`TickType_t` value — 0 means "don't block", `portMAX_DELAY` means
"block indefinitely" (only with `INCLUDE_vTaskSuspend == 1`).

A naive mapping of `After(d)` → `ceil(d · rate_hz)` would violate the
"at least d" contract because `vTaskDelay`-based blocking can return
up to nearly one tick period early.  The same tick-phase problem that
P7B solved for `Clock::delay()` applies to every timed mutex/semaphore
acquisition.

## Decision

### 1. Timeout → native mapping

| Timeout         | Native behaviour                                              |
|-----------------|---------------------------------------------------------------|
| `NoWait`        | `xSemaphoreTake(handle, 0)` — single attempt, no block        |
| `After(ZERO)`   | `xSemaphoreTake(handle, 0)` — same native call; mapped to `Error::Timeout` by the caller |
| `After(d > 0)`  | Absolute-deadline loop with per-chunk guard tick              |
| `Forever`       | Loop of max-finite native takes until acquired                |

`Forever` MUST NOT be mapped to `portMAX_DELAY`.  FreeRTOS only
guarantees indefinite blocking with that sentinel when
`INCLUDE_vTaskSuspend` is set to 1 in `FreeRTOSConfig.h`.  Looping
max-finite chunks avoids adding that configuration requirement.

### 2. `After(d > 0)` — absolute-deadline loop

```rust
let deadline = FreeRtosClock::now()
    .checked_add(duration)
    .ok_or(Error::Overflow)?;

loop {
    // Opportunistic immediate attempt (resource may already be free).
    if take(0) == Acquired { return Ok(Acquired); }

    let now = FreeRtosClock::now();
    if now >= deadline { return Ok(Unavailable); }

    let remaining = deadline - now;
    let payload_ticks = duration_to_ticks_ceil(remaining, tick_rate)?;

    let max_payload = max_finite_ticks - 1;  // reserve for per-chunk guard
    let payload = payload_ticks.min(max_payload as u128);
    let native_ticks = payload + 1;          // per-chunk guard tick (ADR 0023 §4)

    if take(native_ticks as u64) == Acquired {
        return Ok(Acquired);
    }
    // Native wait may have returned early due to tick-phase alignment.
    // Re-read the absolute clock — only timeout when the deadline passes.
}
```

This algorithm guarantees:

- No early timeout: the loop re-checks the monotonic deadline on every
  wake, so a tick-phase early return from one chunk does not cause a
  premature `Timeout`.
- Guard-tick-per-chunk: each native `xSemaphoreTake(ticks)` adds one
  guard tick, compensating for the remainder of the current tick period.
- Absolute monotonic deadline: the loop survives clock adjustments,
  spurious wakeups, and early returns.

### 3. `Forever` — finite-chunk loop

```rust
loop {
    match take(max_finite_ticks) {
        Acquired => return Ok(Acquired),
        TimedOut => continue,   // wake and retry
        Invalid  => fatal,      // invariant violation
    }
}
```

Each iteration blocks for the maximum finite native tick value.  If
the native take times out (returns before acquisition), the loop
simply tries again.  This is semantically equivalent to indefinite
blocking without depending on `INCLUDE_vTaskSuspend`.

### 4. Scheduler-state preconditions

Blocking operations (`After(d > 0)` and `Forever`) MUST check the
scheduler state before entering the wait loop:

| Scheduler state | Error                     |
|-----------------|---------------------------|
| `Running`       | Proceed                   |
| `NotStarted`    | `Error::NotInitialized`   |
| `Suspended`     | `Error::Busy`             |
| `Unknown(_)`    | `Error::Internal`         |

`NoWait` and `After(ZERO)` do NOT require the scheduler to be
`Running`.  A zero-tick `xSemaphoreTake` is a non-blocking check
that works regardless of scheduler state.

### 5. Error mapping

The native take result is a status code, not a `Result<(), Error>`:

| Native result  | Mutex                        | Semaphore          |
|----------------|------------------------------|--------------------|
| `Acquired`     | `Ok(Guard)`                  | `Ok(())`           |
| `Timeout` from `NoWait` | `Error::LockFailed` | `Error::Timeout`   |
| `Timeout` from `After(ZERO)` | `Error::Timeout`   | `Error::Timeout`   |
| `Timeout` from `After(d>0)` | `Error::Timeout`      | `Error::Timeout`   |
| `Invalid`      | `panic!` (invariant)         | `panic!` (invariant) |

The distinction between `LockFailed` and `Timeout` for `NoWait` vs
`After(ZERO)` is intentional: `NoWait` says "I don't want to block at
all", while `After(ZERO)` says "I'm willing to wait for zero time".
POSIX makes the same distinction (ADR 0007).

### 6. ISR context

Mutex and semaphore operations are NOT callable from ISR context in
P7C.  ISR-safe `FromISR` variants are deferred per ADR 0003 and
ADR 0008.  FreeRTOS mutexes are inherently task-context-only.

## Consequences

- `After(d > 0)` never returns `Timeout` before at least `d` has
  elapsed (modulo the guard-tick compensation for the *last* chunk's
  phase alignment — the final early-return window is bounded to < 1
  tick period).
- `Forever` works without `INCLUDE_vTaskSuspend`.
- Scheduler-state violations for blocking ops return typed errors
  rather than panicking, giving the application a recovery path.
- The wait engine is a single module (`wait.rs`) shared by Mutex and
  both Semaphore types — no duplicated timeout logic.
- Tick conversion and chunking reuse P7B's `osal-portable::tick_time`
  and the per-chunk guard-tick algorithm established in ADR 0023.
