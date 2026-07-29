# ADR 0029: FreeRTOS Timer Service Model

## Status

Accepted (2026-07-29)

## Context

The OSAL Timer trait requires creation, start, stop, reset, change-period,
and automatic callback dispatch for both OneShot and Periodic modes.  Key
behavioral requirements:

- Callbacks are **not ISR** — they execute in a service context.
- Callbacks must be able to control their own timer or other timers
  (reentrant safe).
- Periodic timers use **fixed-rate reload**: missed periods coalesce into
  a single callback invocation.
- `change_period()` must not alter the current deadline.
- Last-handle drop must prevent future callbacks without waiting for
  an in-flight callback.
- Clone shares the logical timer; dropping a clone does not affect the
  timer.
- Shutdown must be rejected (`Busy`) when called from within the timer
  service task (self-shutdown deadlock prevention).
- A public Timer handle holds a RuntimeLease; the internal service task
  does not.

FreeRTOS provides native software timers via `xTimerCreate`,
`xTimerStart`, `xTimerStop`, `xTimerReset`, `xTimerChangePeriod`, and
`xTimerDelete`, dispatched by a built-in timer daemon task when
`configUSE_TIMERS == 1`.  This ADR evaluates whether to wrap the native
API or build a custom service.

## Decision

### Reject native FreeRTOS software timer wrapper

Native FreeRTOS software timers are **not** used as the source of truth
for the public OSAL Timer trait.  Four incompatibilities drive this
decision:

1. **Missed-period semantics differ.**  ROUSSATL requires coalescing:
   after N missed periods, only one callback fires, and the deadline
   advances to the first period boundary after `now`.  FreeRTOS
   auto-reload timers experiencing backlog advance expiry and may
   immediately invoke the callback to clear accumulated reload periods.
   These semantics are not configurable.

2. **`change_period()` semantics differ.**  ROUSSATL requires that the
   current deadline is unchanged; only future reloads use the new period.
   FreeRTOS `xTimerChangePeriod()` modifies the timer's period and
   re-evaluates expiry through the timer command queue, which cannot
   guarantee the current-deadline-preserving semantics.

3. **Control operations are asynchronous commands.**  FreeRTOS start,
   stop, reset, change-period, and delete all post commands to the timer
   command queue (`configTIMER_QUEUE_LENGTH`).  Success means the
   command was queued, not that the daemon has applied it.  This
   introduces command-queue-full error paths, blocking constraints inside
   callbacks (cannot block waiting for queue space), and last-handle
   delete/reclaim race conditions.

4. **Callback context reclamation is unsafe.**  `xTimerDelete()` posts a
   delete command; it does not synchronously confirm the daemon has
   released the callback context.  Immediately freeing the Rust closure
   (and its captures) could cause use-after-free if the daemon still
   references it.  Retaining the context indefinitely leaks memory.

### Custom Timer Service Task architecture

P7F implements a ROUSSATL-owned Timer Service Task:

```
FreeRtosTimer                          // Arc<InnerHandle { id, RuntimeLease }>
    ↓ (by ID)
TimerServiceControl                    // static OnceLock<Mutex<ServiceSlot>>
    ↓
TimerService                           // spawned lazily on first start()
    ├── native mutex (state)
    ├── binary wake semaphore
    ├── completion EventGroup
    └── TimerServiceState
          ├── Vec<TimerEntry>
          ├── next_id
          └── stop_requested

TimerEntry
    ├── id: u64
    ├── state: TimerState             // osal-portable pre-advance model
    ├── callback: Option<TimerCallback>
    └── deleted: bool
```

This architecture does **not** require `configUSE_TIMERS == 1`.  It
depends only on the task, mutex, semaphore, EventGroup, and tick
primitives already established by P7C–P7E.

### TimerState as sole deadline/mode source of truth

`osal_portable::TimerState` is reused directly — the same pre-advance
state machine used by POSIX and Mock backends.  This ensures identical
deadline arithmetic, coalescing, and overflow behavior across all
backends.

### Lazy worker creation

The service task is created on the first `start()` or `reset()` call,
not during `initialize()`.  This avoids creating a task that may never
be needed (if no timers are ever started) and sidesteps the problem of a
pending-but-never-scheduled service task during early shutdown.

### Scheduler preconditions

| Operation | NotStarted | Suspended |
|-----------|-----------|-----------|
| `new()` | OK | OK |
| `stop()` | OK | OK |
| `change_period()` (stopped timer) | OK | OK |
| `change_period()` (running timer) | OK | OK |
| `start()` | `NotInitialized` | `Busy` |
| `reset()` | `NotInitialized` | `Busy` |

Rationale: `start()` and `reset()` require a running scheduler to create
the worker task and perform blocking operations.  `stop()` and
`change_period()` only modify in-memory state under a mutex and never
block.

### Binary semaphore wake with Full=success coalescing

The service task blocks on a binary semaphore (not a condvar).  Multiple
state changes between service-loop iterations are coalesced: if the
semaphore is already signaled when `semaphore_give()` is called, the
`GiveStatus::Full` return is treated as success (not `Overflow`), since
the already-pending wake will cause the service to rescan.

### Callback dispatch: take-execute-restore

The service loop follows the same take-execute-restore pattern as POSIX:

1. Lock registry mutex.
2. Find the earliest expired timer by `(deadline, id)`.
3. Call `TimerState::advance_on_expiry(now)` — pre-advances state.
4. `take()` the callback out of the entry.
5. Unlock registry mutex.
6. Execute `callback()` — **outside all locks**.
7. Re-lock registry mutex.
8. If the entry still exists and is not deleted, restore the callback.
   If deleted, drop the callback outside the lock.

The `(deadline, id)` selection key prevents short-period timers from
starving same-deadline timers (the stable ID serves as the tiebreaker).

### Deadline waiting: chunked semaphore_take

The service must wait until the earliest deadline OR until a control
operation signals the wake semaphore:

```
loop:
    now = Clock::now()
    if now >= deadline: return DeadlineReached
    remaining = deadline - now
    chunk = min(remaining_ticks + guard_tick, max_finite_payload)
    semaphore_take(wake, chunk):
        Acquired → return StateChanged
        Timeout  → re-read now, loop
        Invalid  → fatal panic
```

Each finite chunk includes a guard tick (ADR 0023 §5).  Timeout always
re-reads `Clock::now()` — the service does not trust that exactly
`chunk` ticks elapsed.

### Clone and last-handle drop

`FreeRtosTimer` is `Clone` via `Arc<InnerHandle>`.  The inner handle
holds only `id` and `RuntimeLease`.  Callbacks live in the registry, not
in the handle.

On last-handle drop:

1. Lock registry mutex.
2. Mark entry `deleted = true`.
3. Call `TimerState::stop()`.
4. `take()` the callback if not currently in flight.
5. Remove entry from the vector.
6. Unlock.
7. Signal wake semaphore.
8. Drop callback (and its captures) **outside the lock**.
9. `RuntimeLease` drops.

If the callback is currently executing (taken by `dispatch_one`), step 4
finds `callback: None`.  When `dispatch_one` finishes and attempts to
restore the callback, it observes `deleted == true` and does not restore
it.  The callback is instead dropped outside the lock by `dispatch_one`.

### Shutdown lifecycle

**Self-call detection:** `shutdown()` compares the calling thread's
native task identity to the service worker's handle.  If they match,
returns `Error::Busy` — the worker cannot wait for its own completion
EventGroup.

**Normal shutdown path:**

1. `begin_shutdown()` — requires zero `RuntimeLease` (no public handles).
2. Lock control mutex + registry mutex.
3. Verify `timers.is_empty()` — returns `Busy` if any timers still
   registered.
4. Set `stop_requested = true`.
5. Signal wake semaphore.
6. Unlock registry mutex + control mutex.
7. Wait on completion EventGroup (service task sets this bit before
   self-deleting).
8. Lock control mutex.
9. Delete wake semaphore, registry mutex, completion EventGroup.
10. Transition slot to `Stopped`.
11. Commit shutdown.

**Failure-atomic:** If any step fails before `stop_requested` is set
(including the `timers.is_empty()` check or the scheduler-state check),
all state is preserved and the runtime remains `Running`.  A `Busy`
error from a timer shutdown precondition leaves the timer service fully
operational.

### Lock ordering

```
Timer API:       control mutex → registry mutex
shutdown phase1: control mutex → registry mutex → signal worker
                 release both locks
shutdown phase2: wait completion EventGroup (outside all locks)
shutdown phase3: control mutex → delete resources → Stopped
worker loop:     only registry mutex
callback:        holds neither lock
```

### Internal service task identity

The service task is created via raw `xTaskCreate` — it does **not**:

- Acquire a public `RuntimeLease`
- Set OSAL Task TLS identity
- Increment `Task::count()`
- Produce a `FreeRtosTask` handle

From `Task::current()`, the service context returns `None`.  This
matches POSIX, where the timer worker pthread is not an OSAL-created
Task.

Internally, the service task retains a `NativeTaskHandle` obtained from
`xTaskGetCurrentTaskHandle()` during worker startup.  This handle is
used solely for self-shutdown detection.

### Host fixture strategy

The host fixture reuses the real `TimerService` Rust code.  Only the
sys-layer primitives are replaced:

- `internal_task_create` → `std::thread::spawn` with `JoinHandle` tracking
- Native mutex → `std::sync::Mutex` + `Condvar`
- Binary semaphore → `Mutex<bool>` + `Condvar`
- EventGroup → `Mutex<u32>` + `Condvar`
- `Clock::now()` → virtual tick counter advanced by `delay_ticks()`

Fixture reset ordering matches P7E: join all internal threads → assert
zero blocked waiters → clear maps → reset atomics.

## Consequences

- `configUSE_TIMERS` is **not** required.  P7F is independent of the
  FreeRTOS native software timer subsystem.
- The timer service task adds one internal task per runtime session
  (lazily created).  Memory overhead: one TCB, one stack (~4096 bytes),
  plus the wake semaphore and completion EventGroup.
- Callback execution latency is bounded by the service loop scanning
  interval (gated by tick resolution and any long-running callbacks).
- Real-time guarantees require `configMAX_PRIORITIES - 1` service task
  priority, which may preempt application tasks.  Applications needing
  hard real-time timer dispatch should evaluate this trade-off.
- The custom service avoids all native timer command-queue races,
  enabling deterministic last-handle drop and safe Rust callback
  reclamation.
- The `InternalTaskHandle` API added to `-sys` is reusable for future
  internal services (e.g. an ISR dispatch task).
- Real-kernel tick-interrupt timer validation remains explicitly
  deferred.  P7F is host-contract-verified only.

### MUST rules

- MUST use `TimerState` as the deadline/mode source of truth.
- MUST execute callbacks outside all service locks.
- MUST pre-advance `TimerState` before callback invocation.
- MUST coalesce missed periodic expirations into one callback.
- MUST NOT retain a strong public `Timer` handle in the registry.
- MUST prevent future callbacks after last public-handle drop.
- MUST NOT wait for an in-flight callback in `Timer` handle `Drop`.
- MUST drop callbacks and captured objects outside the registry lock.
- MUST reject service self-shutdown with `Error::Busy`.
- MUST NOT depend on native FreeRTOS software timer auto-reload semantics.
