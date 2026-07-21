# ADR 0019: Backend Runtime Ownership Without BSP

## Status

Accepted (2026-07-21)

## Context

ADR 0015 defines a four-state runtime lifecycle (Uninitialized →
Initializing → Running → ShuttingDown → Uninitialized) with an
active-object lease counter. ADR 0016 refines it into a single
`AtomicUsize` CAS state machine. ADR 0014 establishes the Backend /
BSP boundary in principle. BSP crates (`osal-bsp`, `osal-bsp-linux`)
exist as deferred workspace placeholders, but no BSP capability model
or concrete platform integration is currently selected by the facade.

We now need to decide **where** the `RuntimeLifecycle` instance lives,
how backends wire it into their service lifecycles, and which objects
must hold a `RuntimeLease` — all without introducing BSP code that
would need to be unwound later.

## Decision

### 1. Each backend independently owns a `RuntimeLifecycle`

```
osal-backend-posix
└── static POSIX_RUNTIME: RuntimeLifecycle

osal-backend-mock
└── static MOCK_RUNTIME: RuntimeLifecycle
```

`osal-shared` provides only the mechanism:

- `RuntimeLifecycle` — the packed state+count word and CAS operations
- `RuntimeLease<'a>` — an RAII guard decrementing the count on drop
- `InitializeTransition<'a>` — commit/rollback guard for initialization
- `ShutdownTransition<'a>` — commit/rollback guard for shutdown

**Rationale.** Placing a single `static RUNTIME` in `osal-shared`
would couple the two backends — a test linking both crates (e.g. a
future integration harness) would share one lifecycle, and Mock's
initialize/shutdown would interfere with POSIX tests.  Independent
statics let each backend maintain its own state, matching the reality
that exactly one backend is active in any process.

### 2. Initialization order does not yet include BSP

```
begin_initialize()          // CAS Uninitialized → Initializing
→ backend internal services // currently only TimerService for POSIX
→ commit()                  // CAS Initializing → Running
```

```
begin_shutdown()            // CAS Running,0 → ShuttingDown,0
→ backend internal services
→ commit()                  // CAS ShuttingDown → Uninitialized
```

When a real BSP crate arrives (future), the order becomes:

```
BSP initialize → backend initialize
```

The public `osal::initialize()` API does not change — only the
internal wiring in each backend's `runtime.rs` gains an additional
call.  This means **no BSP crate, module, or trait is introduced
now**; the extension point is structural.

### 3. Managed-object classification

| Object | Holds `RuntimeLease`? | Rationale |
|--------|----------------------|-----------|
| `Timer` (PosixTimer, MockTimer) | **Yes** | Creates a backend timer entry; must block shutdown |
| `Queue` | **Yes** | Allocates backend storage; must block shutdown |
| `Mutex` | **Yes** | Allocates a native mutex; must block shutdown |
| `CountingSemaphore` | **Yes** | Allocates a native semaphore; must block shutdown |
| `BinarySemaphore` | **Yes** | Allocates a native semaphore; must block shutdown |
| `Task` handle (inner) | **Yes** | Represents a logical task object; dropped handle → join/detach |
| `Clock` | **No** (deferred) | Stateless query; no backend resource to protect |
| `System` | **No** (deferred) | Stateless query + critical-section entry; no owned resource |

`Clock` and `System` are excluded for now because their APIs are
stateless queries or transient operations that do not create or
destroy backend resources.  If future extensions add resource
ownership (e.g. a clock calibration context), they can gain a
lease in that slice — without breaking existing callers because
`NotInitialized` is a new error variant those callers already
can't receive.

### 4. Task's two counts remain distinct

```
Task::count()                  — entries whose function body has not yet completed
RuntimeLifecycle::active_objects() — Task handle inners still alive (not dropped)
```

A finished Task whose handle is still held:

```
Task::count()       == 0   // entry has returned
active_objects()    == 1   // handle not yet dropped
shutdown()          → Busy // object still alive
```

The existing `LIVE_COUNT` atomic in `PosixTask` and `LiveTaskToken`
RAII guard are **not** merged with `RuntimeLease`.  They serve
different purposes: `LIVE_COUNT` tracks execution-in-flight for
`Task::count()`, while `RuntimeLease` tracks handle lifetime for
shutdown gating.

### 5. Internal services do not hold leases

| Component | Holds `RuntimeLease`? |
|-----------|----------------------|
| `TimerService` worker thread | No |
| `TimerServiceControl` block | No |
| `TaskTlsSlot` | No |
| Backend mutex/condvar primitives (inside `sys/`) | No |

Only **user-visible logical objects** contribute to
`active_objects()`.  If internal services held leases, the count
would never reach zero and shutdown would always return `Busy`.

### 6. Constructor contract

Every managed-object constructor must follow this order:

```
1. validate_parameters()?       // Error precedence: parameters > runtime state
2. runtime::acquire_object()?   // → RuntimeLease, or NotInitialized
3. create_native_resource()?    // backend-specific creation
4. construct Inner { _runtime, native }
```

If step 3 fails, the local `RuntimeLease` is dropped, which
decrements the count — no leak.  If step 3 succeeds, the lease
moves into the inner `Arc` and is released when the last clone
drops.

### 7. Clone semantics

Cloning a managed-object handle clones the `Arc<Inner>` but does
**not** call `acquire_object()` again.  One logical object, one
`RuntimeLease`, regardless of how many handles point to it.

Only when the last `Arc` is dropped does the `RuntimeLease`
decrement the active-object count.

## Consequences

- `osal-shared` does **not** gain a global `RuntimeLifecycle`
  static; it remains a pure mechanism crate.
- Each backend gains a `static RUNTIME: RuntimeLifecycle` and a
  public `runtime` module exposing `initialize()`, `shutdown()`,
  `state()`, `acquire_object()` (pub(crate)), and
  `active_objects()` (testkit-gated).
- POSIX `timer_service` lifecycle is orchestrated by the backend
  `runtime.rs`: `RuntimeLifecycle` first, then timer service.
  Timer-local liveness checks remain as defense-in-depth even
  though `PosixTimer` handles now hold `RuntimeLease` (P6B-6A).
- `Error::NotInitialized` becomes reachable from every managed-object
  constructor; `Error::AlreadyInitialized` and `Error::Busy` become
  reachable from `initialize()` and `shutdown()`.
- Mock backend can inject faults into `initialize`/`shutdown` hooks
  to verify rollback semantics — something POSIX cannot easily do.
  **Deferred:** runtime hook failure injection is not yet implemented;
  the current Mock `runtime.rs` calls `reset_runtime()` directly
  without a failure seam.
- Task `LIVE_COUNT` and `RuntimeLease` are separate mechanisms;
  documentation must clearly distinguish them.
- ADR 0015's "Object lease tracking" section is now fully realized:
  the lease mechanism exists in `osal-shared`, and this ADR assigns
  ownership and integration responsibility to each backend.
