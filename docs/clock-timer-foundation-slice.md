# Clock and Timer Foundation Slice

## Status

Complete — Clock and Timer are implemented across the full stack
(host-contract-verified on all three backends: Mock, POSIX, FreeRTOS).

## Architecture

```
                 osal (facade)
                     |
    +----------------+----------------+
    |                |                |
osal-backend-  osal-backend-   osal-backend-
  posix           mock           freertos
    |                |                |
PosixClock      MockClock        FreeRtosClock
PosixTimer      MockTimer        FreeRtosTimer
PosixTimerSvc  MockTimeRuntime   FreeRtosTimerSvc
(pthread)      (RefCell<Dur>)    (xTaskCreate + Semaphore)
```

## Clock Model

| Backend | Source | `delay()` |
|---------|--------|-----------|
| POSIX | `clock_gettime(CLOCK_MONOTONIC)` | EINTR-loop `nanosleep` |
| Mock | `MockTimeRuntime` virtual counter | Advance + dispatch timers |
| FreeRTOS | `vTaskSetTimeOutState()` coherent snapshot | Per-chunk guard-tick `vTaskDelay` |

## Timer Service Model

| Aspect | POSIX | Mock | FreeRTOS |
|--------|-------|------|----------|
| Service | Single detach pthread | Synchronous in `advance_clock` | Lazy xTaskCreate (internal) |
| Registry | `static mut` Arc-protected | `Vec` in `MockTimeRuntime` | `Vec<TimerEntry>` + native mutex |
| Callback | `pthread` context, outside lock | Outside `RefCell` borrow | Worker task, outside all locks |
| Wake | `pthread_cond_signal` | N/A (sync dispatch) | Binary semaphore (Full=coalesce) |
| Completion | `pthread_join` | N/A | EventGroup (sticky bit) |

## Contract Tests Passing

- **Clock Basic** (all backends): now monotonic, elapsed non-negative, delay(0) immediate
- **Clock Controlled** (Mock): advance increases now/elapsed
- **Timer Core** (all backends): 6 shared core contract cases
- **Timer Controlled** (Mock + FreeRTOS): 5 deterministic controlled contract cases
- **Timer Realtime** (POSIX): timing-bounds contract cases
- **FreeRTOS-specific**: TimerState semantics (change_period, reset, fixed-rate,
  coalescing), callback reentry, drop/shutdown lifecycle races, failure-atomic
  rollback, scheduling and finite-chunk wait coverage

## Intentionally Deferred

- ISR timers (FreeRTOS extension)
- Timer priority
- Callback thread pool
- Strict cross-timer ordering

## Next Steps

1. ISR Timer extensions (FreeRTOS)
2. Real-kernel timer validation (QEMU / physical MCU) → P7G
