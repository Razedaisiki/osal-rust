# ADR 0018: POSIX Timer Service Lifecycle

## Status

Accepted (2026-07-21)

## Context

The current POSIX timer service uses `pthread_once` to lazy-initialise
a detached permanent worker thread.  This makes it impossible to stop,
join, or restart the service.  For the runtime lifecycle (ADR 0015),
backend services must support explicit initialise / shutdown /
re-initialise.

## Decision

### Worker thread

- The worker thread is **joinable** (not detached).
- The service holds `Arc<TimerService>`.  The worker is given a clone
  via `Arc::into_raw` + `Arc::from_raw`.
- On shutdown, `stop_requested` is set, the condvar is broadcast, the
  worker exits, and `pthread_join` is called.
- The worker must not call `shutdown()` on itself (self-join).

### Control block vs. service instance

A process-lifetime `PosixInitCell<TimerServiceControl>` holds a
permanent mutex and slot.  The slot is `Stopped`, `Running { service,
worker }`, or `Stopping`.

The actual `TimerService` (timers, condvar, state) is created on
`initialize()` and destroyed on `shutdown()`.  The control block
persists across restarts.

### Lock ordering

```
Timer API:       control mutex → service mutex
shutdown:        control mutex → service mutex
worker loop:     only service mutex
callback:        holds neither lock
```

`service → control` is forbidden.

### Callback execution

Callbacks execute outside the service mutex.  Callbacks must not
panic or unwind (`panic = "abort"` is the workspace default).

### Shutdown / re-initialise

| State     | `initialize()`              | `shutdown()`       |
|-----------|----------------------------|--------------------|
| `Stopped` | create service + worker    | `NotInitialized`   |
| `Running` | `AlreadyInitialized`       | stop + join worker |
| `Stopping`| `Busy`                      | `Busy`             |

`shutdown()` returns `Busy` while any live `Timer` handle exists
(checked under the service mutex after marking `stop_requested`).

### Timer API errors

All service functions (`register`, `start`, `stop`, `reset`,
`change_period`, `deregister`) return `Result` instead of silently
ignoring errors.  `PosixTimer` propagates these to the caller.
`Deregister` on drop uses `debug_assert!` (drop cannot return an
error).

### No `catch_unwind`

The callback panic strategy follows the platform panic policy
(`abort`).  No `catch_unwind` is used — it requires `std` and is
not meaningful in a `no_std` backend.

## Consequences

- `pthread_once` is only used for the permanent control block.
- The service instance is explicitly created and destroyed.
- Timer API signatures change from `-> ()` / `-> Option<u64>` to
  `-> Result<()>` / `-> Result<u64>`.
- `PosixTimer` propagates real errors instead of always returning
  `Ok(())`.
- Integration tests can stop and restart the timer service between
  test cases.
