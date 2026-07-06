# ADR 0010: Timer Execution Model

## Status

Accepted (2026-07-06)

## Context

P3 implements the `Timer` trait with OneShot and Periodic modes. Key
design decisions include: callback type, execution context, scheduling
policy for periodic timers, re-entrancy guarantees, handle sharing,
and the interaction between timer control operations and in-flight
callbacks.

## Decision

### Callback type

`TimerCallback` is changed from `Box<dyn Fn() + Send + 'static>` to
`Box<dyn FnMut() + Send + 'static>`. Callbacks may maintain internal
mutable state. A callback is never called concurrently with itself.

### Execution context

Callbacks execute in a **dedicated timer service context**, not in the
caller of `start()`/`reset()`, not in ISR context, and not while
holding any timer management lock. Callbacks may call other OSAL APIs.

- Mock: callback is invoked synchronously inside `advance_clock()` /
  `delay()`, but outside the runtime's internal lock.
- POSIX: callback is invoked by a single background `pthread` (Timer
  Service Thread), outside the registry mutex.

### Scheduling policy

**Fixed-rate with missed expiration coalescing.** Each periodic timer
has a `next_deadline` based on the *scheduled* deadline, not the
callback completion time. If multiple periods are missed (e.g., system
was busy), only one callback fires and the next deadline is advanced to
the first multiple of `period` that is strictly after `now`.

### Generation counter

Every state-changing operation (`start`, `stop`, `reset`,
`change_period`, last-handle `drop`) increments a `generation` counter.
Before executing a callback, the current generation is recorded in an
`ExpirationToken`. After the callback completes, if the generation has
changed, the timer's state has been modified and the old expiration
logic (e.g., auto-reload for Periodic) is skipped.

### Handle model

`Timer` requires `Clone`. Mock uses `Rc`, POSIX uses `Arc`. All clones
share the same timer. The last handle drop cancels future callbacks
(does not wait for in-flight callbacks). The timer service registry
holds weak references, not strong handles.

### OneShot semantics

Fires once, then transitions to stopped. If `start()`/`reset()` is
called during callback execution, the new state takes precedence.

### Periodic semantics

Fires, then auto-reloads `next_deadline = previous_deadline + period`.
Missed periods are coalesced. If `stop()` is called during callback,
auto-reload is skipped.

### Non-requirements

- No ISR timers (deferred to FreeRTOS extension).
- No callback priority or callback thread pool.
- No strict fairness or ordering guarantees across different timers.
- No `is_running()` or `period()` query methods.

## Rationale

- `FnMut` allows stateful callbacks (counters, accumulators) without
  requiring `Cell`/`RefCell` wrappers.
- Single service thread (POSIX) limits thread count and matches FreeRTOS
  Timer Daemon model.
- Fixed-rate scheduling is predictable and matches common RTOS behavior.
- Generation counter is a well-known pattern for preventing
  stale-state-overwrite bugs in async timer implementations.

## Consequences

- `Timer` trait gains a `Clone` bound.
- All existing `TimerCallback` usage must change from `Fn()` to `FnMut()`.
- `MockClock` must be integrated with a `MockTimeRuntime` that manages
  timer dispatch.
- POSIX requires a lazy-initialized singleton `TimerService` thread.
