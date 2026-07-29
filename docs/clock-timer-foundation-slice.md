# Clock and Timer Foundation Slice

## Status

Clock is implemented across the full stack.  Timer is implemented on
Mock and POSIX (Validated).  FreeRTOS Timer architecture is implemented;
lifecycle and callback fixes are in stabilization (P7F).

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

- **Clock Basic** (Mock + POSIX + FreeRTOS): now monotonic, elapsed non-negative, delay(0) immediate
- **Clock Controlled** (Mock): advance increases now/elapsed
- **Timer Core** (Mock + POSIX + FreeRTOS): 6 tests (zero period, stopped, stop idem, change_period zero, clone, drop)
- **Timer Controlled** (Mock): 5 tests (OneShot, Periodic, stop, reset, coalescing)
- **Timer Realtime** (POSIX): 4 tests (OneShot bounds, Periodic ≥2, stop, reset delays)
- **Timer Lifecycle** (FreeRTOS): 7 backend-specific tests (OneShot, Periodic, self-stop, clone, last-drop, non-last clone)

## Intentionally Deferred

- ISR timers (FreeRTOS extension)
- Timer priority
- Callback thread pool
- Strict cross-timer ordering

## Next Steps

1. ISR Timer extensions (FreeRTOS)
2. Controlled timer tests for FreeRTOS (virtual-tick-aware semaphore wait)
3. Real-kernel timer validation (QEMU / physical MCU)
