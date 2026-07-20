# ADR 0015: Runtime Lifecycle

## Status

Accepted (2026-07-20).  **The two-atomic double-check lease algorithm
in the original text has been superseded by ADR 0016.**  The four-state
model, transaction guards, re-initialisability, and error semantics
remain valid.

## Context

OSAL currently has no explicit initialisation or shutdown path.
Backend services such as the POSIX timer service initialise
lazily via `pthread_once` and run a detached permanent thread.
Objects can be created at any time without a guard. There is no
way to cleanly stop and restart the system — for example, between
integration tests or during a controlled system restart.

## Decision

### State machine

```
Uninitialized ── initialize() ──→ Initializing
     ↑                               │
     │                          success │ failure
     │                               ↓       ↓
     │                           Running   Uninitialized
     │                               │
     │                        shutdown()
     │                               │
     │                        ShuttingDown
     │                               │
     │                     success │     │ failure
     │                            ↓       ↓
     └─────────────────── Uninitialized   Running
```

No permanent `Terminated` state — this allows re-initialisation
within the same process (essential for integration tests).

### State transition rules

| Operation       | Current state              | Result |
|----------------|---------------------------|--------|
| `initialize()` | `Uninitialized`           | enter `Initializing` |
| `initialize()` | `Running`                 | `Error::AlreadyInitialized` |
| `initialize()` | `Initializing`, `ShuttingDown` | `Error::Busy` |
| `shutdown()`   | `Running`, count == 0     | enter `ShuttingDown` |
| `shutdown()`   | `Running`, count > 0      | `Error::Busy` |
| `shutdown()`   | `Uninitialized`           | `Error::NotInitialized` |
| `shutdown()`   | `Initializing`, `ShuttingDown` | `Error::Busy` |
| Init failure   | `Initializing`            | rollback to `Uninitialized` |
| Shutdown fail  | `ShuttingDown`            | rollback to `Running` |
| Shutdown ok    | `ShuttingDown`            | transition to `Uninitialized` |
| `acquire()`    | `Running`                 | `Ok(RuntimeLease)` |
| `acquire()`    | any other                 | `Error::NotInitialized` |

Backend and BSP lifecycle hooks must be **failure-atomic**: if
`initialize()` returns an error the component must remain
uninitialised; if `shutdown()` returns an error the component
must still be fully operational. The `RuntimeLifecycle` guard
publishes the outcome but does not repair partially-stopped
services.

### Object lease tracking

Each OSAL object holds a `RuntimeLease` (RAII guard) that
increments an atomic active-object counter on creation and
decrements it on drop. Cloned handles share the same inner
state and do not create additional leases.

`shutdown()` checks the counter before proceeding. If non-zero,
it returns `Error::Busy`.

> **Superseded.** The two-atomic double-check algorithm above has been
> replaced by ADR 0016, which uses a single `AtomicUsize` packing
> state and count into one word.  The single-word CAS provides a
> unique linearisation point for each of `acquire` and `shutdown`,
> closing the window where a temporary count increment could allow
> shutdown to proceed incorrectly.

Internal runtime services (timer service thread, backend control
blocks, BSP console) do **not** hold [`RuntimeLease`]s — only
user-visible logical objects (Queue, Mutex, Task handle, Timer)
contribute to the active-object count.  Without this distinction,
the runtime would always see a non-zero count and shutdown would
never succeed.

### New error variant

```rust
/// The runtime or resource is currently busy (objects still alive).
Busy,
```

This is a normal, expected error — not `Internal`.

### Object creation guards

Every constructor must:
1. Validate parameters first (error precedence: parameters >
   runtime state).
2. Acquire a runtime lease.
3. Create backend resources.

### Backend services

Backend runtime services (timer service, etc.) must support:
- Explicit `initialize()` — no lazy `pthread_once` for the
  service itself.
- Explicit `shutdown()` — stop thread, join, release resources.
- Re-initialisation after shutdown.

## Consequences

- `osal-shared` gains `runtime` module with `RuntimeLifecycle` and
  `RuntimeLease`.
- Every object inner struct gains a `_runtime: RuntimeLease` field.
- POSIX timer service is refactored from detached permanent thread
  to joinable owned service with explicit start/stop.
- All existing constructors gain a runtime lease acquisition step.
- `Error::Busy` added to `osal_api::error::Error`.
- Runtime contract tests verify initialisation, shutdown, lease,
  and error precedence across both backends.
