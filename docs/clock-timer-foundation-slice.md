# Clock and Timer Foundation Slice

## Status

Complete — Clock and Timer are implemented across the full stack.

## Architecture

```
                 osal (facade)
                     |
         +-----------+-----------+
         |                       |
  osal-backend-posix    osal-backend-mock
         |                       |
    PosixClock              MockClock
    PosixTimer             MockTimer
    PosixTimerService    MockTimeRuntime
    (pthread)            (RefCell<Duration>)
```

## Clock Model

| Backend | Source | `delay()` |
|---------|--------|-----------|
| POSIX | `clock_gettime(CLOCK_MONOTONIC)` | EINTR-loop `nanosleep` |
| Mock | `MockTimeRuntime` virtual counter | Advance + dispatch timers |

## Timer Service Model

| Aspect | POSIX | Mock |
|--------|-------|------|
| Service | Single detach pthread | Synchronous in `advance_clock` |
| Registry | `static mut` Arc-protected | `Vec` in `MockTimeRuntime` |
| Callback | `pthread` context, lock-held | Outside `RefCell` borrow |
| Wake | `pthread_cond_signal` | N/A (sync dispatch) |

## Contract Tests Passing

- **Clock Basic** (Mock + POSIX): now monotonic, elapsed non-negative, delay(0) immediate
- **Clock Controlled** (Mock): advance increases now/elapsed
- **Timer Core** (Mock + POSIX): 6 tests (zero period, stopped, stop idem, change_period zero, clone, drop)
- **Timer Controlled** (Mock): 5 tests (OneShot, Periodic, stop, reset, coalescing)
- **Timer Realtime** (POSIX): 4 tests (OneShot bounds, Periodic ≥2, stop, reset delays)

## Intentionally Deferred

- ISR timers (FreeRTOS extension)
- Timer priority
- Callback thread pool
- Strict cross-timer ordering

## Next Steps

1. Task Foundation Slice
2. FreeRTOS backend
