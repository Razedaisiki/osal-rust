# ADR 0003: ISR API Separation from Core Traits

## Status

Accepted (2026-07-03)

## Context

The initial Queue trait included `isr_send` and `isr_recv` methods, and
the behavior contract specified `isr_lock` for Mutex. However:

- POSIX has no true ISR context. ISR methods would be implemented as
  `lock(NoWait)` or `send(NoWait)`, which involves `pthread_mutex_lock`
  and is not guaranteed to be non-blocking.
- Mock has no interrupt model. ISR methods are trivial wrappers that
  provide no additional test value.
- FreeRTOS genuinely needs ISR-safe variants with `FromISR` suffix and
  `BaseType_t` return for `higher_priority_task_woken`.

Keeping ISR methods on the core traits forces every backend to either
fake ISR support or return `Error::Unsupported`, creating a misleading
API surface.

## Decision

ISR-safe operations are **removed** from the core `Queue`, `Mutex`, and
`Semaphore` traits during P0.

Future FreeRTOS integration will introduce separate extension traits:

```rust
pub trait IsrQueue {
    fn send_from_isr(&self, data: &[u8]) -> Result<IsrWake>;
    fn recv_from_isr(&self, buffer: &mut [u8]) -> Result<IsrWake>;
}
```

## Rationale

- Each backend only implements the traits it can genuinely support.
- POSIX and Mock do not need to carry dead ISR code.
- The core traits stay minimal and correct for all current backends.
- Extension traits can be added without breaking existing code (new impls
  on existing types).

## Consequences

- `Queue` trait loses `isr_send()` and `isr_recv()`.
- `Mutex` trait never had `isr_lock()`; the contract doc now matches.
- `Semaphore` trait ISR methods are deferred (not yet implemented, not
  yet removed — this will be addressed when Semaphore is implemented).
- Mock and POSIX backends remove their ISR method implementations.
- Contract tests for ISR are removed from the core test suite.
