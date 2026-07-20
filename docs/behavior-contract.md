# OSAL Behavior Contract

## 1. Purpose

This document defines the precise behavioral contract that every OSAL
backend must fulfill. It serves three audiences:

- **API designers** (Phase 2): derive trait signatures from the
  contracts below.
- **Backend implementors**: know exactly what each method must do.
- **Test authors**: derive test cases directly from the pre/post
  conditions and error tables.

The contract describes **what** correct behavior looks like, not
**how** backends achieve it. Two backends may use completely different
internal mechanisms as long as the observable behavior matches.

---

## 2. Runtime Model

OSAL public APIs are designed for `no_std` environments.

OSAL requires an allocator for its dynamic object model (boxed
callbacks, dynamic queues, task arguments, runtime registries).
Allocation support is a **project-level runtime assumption**, not an
optional Cargo feature. Every crate that needs heap allocation
declares `extern crate alloc` unconditionally.

Rust `std` support, when available, enables optional host-oriented
integrations only (e.g. `impl std::error::Error`). It does **not**
determine backend availability. POSIX and FreeRTOS backends remain
`no_std` while binding to platform-native facilities through FFI or
platform crates (`libc`, RTOS C APIs).

The `std` Cargo feature is reserved for future host-only capabilities.
It is not required to build, test, or use any backend.

---

## 3. Non-goals

The following are explicitly **not** covered by this contract:

- Performance characteristics (latency, throughput)
- Real-time guarantees (deadline scheduling, priority inversion
  prevention)
- Memory layout or allocation strategy
- Debugging or introspection beyond the listed methods
- Interoperability between different backends in the same process
- Safety against misuse from ISR context (the caller is responsible
  for calling the correct variant)

Backends may provide additional capabilities beyond this contract,
but portable application code must not depend on them.

---

## 4. Backend selection model

An OSAL application links against exactly **one** backend at compile
time. The backend is selected via a Cargo feature flag on the `osal`
facade crate:

```toml
[dependencies]
osal = { version = "0.1" }                    # POSIX (default)
osal = { version = "0.1", default-features = false, features = ["backend-mock"] }  # Mock
```

Only one backend feature may be active. Attempting to enable multiple
backends produces a compile error.

All public types (Mutex, Queue, Task, etc.) resolve to the active
backend's concrete implementation. Application code never imports
backend crates directly.

---

## 5. Common error semantics

All fallible operations return `Result<T, osal_api::Error>`.

### Error variants

| Variant | Meaning | Typical cause |
|---------|---------|---------------|
| `OutOfMemory` | Allocation failed | Heap exhausted |
| `Timeout` | Operation exceeded time limit | `Timeout::After(d)` expired |
| `QueueFull` | Queue at capacity | Non-blocking send on full queue |
| `QueueEmpty` | Queue has no messages | Non-blocking recv on empty queue |
| `QueueClosed` | Queue has been explicitly closed | Send after close; recv on closed empty queue |
| `InvalidMessageSize` | Queue message size mismatch | send/recv buffer length != msg_size |
| `LockFailed` | Could not acquire lock | Mutex held by another context |
| `NotFound` | Resource not found | Invalid handle or ID |
| `Overflow` | Arithmetic overflow or count at max | capacity * msg_size overflow; semaphore at max_count |
| `InvalidParameter` | Argument out of valid range | Overlong name, NUL in name, zero stack, count > max |
| `AlreadyInitialized` | Resource already created/started | Double `spawn()` on a Task |
| `NotInitialized` | Resource not yet started | Future extension (not used by Task) |
| `Unsupported` | Backend cannot perform operation | Mock Forever on full/empty queue |
| `Internal(&'static str)` | Unexpected native error | errno, FreeRTOS status code |

### Rules

1. Every error path listed in this contract must produce the exact
   variant specified — backends must not substitute a different
   variant.
2. The `Internal` variant is a last resort for platform errors with no
   obvious mapping. It must carry a static string identifying the
   source (e.g. `"pthread_mutex_lock: EINVAL"`).
3. `Timeout` is returned only when a time-bounded wait expires without
   success. It is not returned for immediate failures like queue-full.
4. The `Error` type carries no lifetime parameter and no heap-allocated
   data (other than `Internal(&'static str)`).

### Error precedence

When a single operation satisfies multiple error conditions
simultaneously, backends must return errors in the following order of
precedence (highest first):

```
1. Input parameter validation  (InvalidParameter, InvalidMessageSize)
2. Object state validation     (QueueClosed, NotInitialized)
3. Current resource state      (QueueFull, QueueEmpty, LockFailed)
4. Wait and timeout            (Timeout)
5. Backend system errors       (Internal)
```

For Queue operations specifically:

```
InvalidMessageSize
    ↓
QueueClosed
    ↓
QueueFull / QueueEmpty
    ↓
Timeout
    ↓
Internal
```

This means a `send()` to a closed queue with wrong message size must
return `Error::InvalidMessageSize`, not `Error::QueueClosed`. Parameter
validation always takes priority over object state.

---

## 6. Time and timeout semantics

### Primary types

```rust
use core::time::Duration;

pub enum Timeout {
    NoWait,            // return immediately, never block
    After(Duration),   // block for at most this duration
    Forever,           // block indefinitely
}
```

`core::time::Duration` is available in `no_std` and serves as the
universal time representation across the OSAL API.

### Timeout behavior

| Timeout | Behavior |
|---------|----------|
| `NoWait` | The call must return immediately. If the operation would block, return the appropriate error (`QueueFull`, `QueueEmpty`, `LockFailed`, `Timeout`). |
| `After(d)` | Block until success or `d` has elapsed, whichever comes first. If `d` expires, return `Error::Timeout`. The call must not return with `Timeout` before `d` has elapsed (no spurious early wakeups). It may return later due to scheduling. |
| `Forever` | Block until success or a fatal error. Must not return `Error::Timeout`. |

### `After(Duration::ZERO)` semantics

`Timeout::After(Duration::ZERO)` is distinct from `Timeout::NoWait`:

| Timeout | Queue full/empty result |
|---------|------------------------|
| `NoWait` | `QueueFull` or `QueueEmpty` |
| `After(Duration::ZERO)` | `Error::Timeout` |

- `NoWait` queries the immediate resource state and reports the specific
  condition.
- `After(d)` represents a time-bounded wait; a zero-duration wait
  expires instantly if the resource is not immediately available.
- If the resource is available, both `NoWait` and `After(ZERO)` succeed
  immediately.

### Clock contract

- `Clock::now()` returns a monotonically increasing `Duration` from an
  arbitrary epoch (typically process start or system boot).
- The clock must never jump backward.
- Resolution is backend-dependent; portable code must not assume
  sub-millisecond precision.
- `Clock::elapsed(since: Duration) -> Duration` has a default
  implementation: `Self::now().saturating_sub(since)`. Backends only
  need to implement `now()` and `delay()`.
- All OSAL primitives (Queue, Mutex, Semaphore, Timer) use the same
  monotonic time domain provided by the active backend.
- **POSIX**: `clock_gettime(CLOCK_MONOTONIC)`. Wall-clock changes
  must not affect OSAL timing.
- **Mock**: virtual `Duration` counter in `MockTimeRuntime`.
  `delay()` advances the clock AND dispatches any timers that expire
  during the advance. `reset_clock()` is test-only, not part of the
  public `Clock` trait.

### Delay contract

- `Clock::delay(d: Duration)` blocks the calling task for **at least**
  `d`. It may block longer due to scheduling.
- `delay(Duration::ZERO)` must return immediately.
- POSIX uses `nanosleep` with `EINTR` restart (remaining time preserved).
- Deadline arithmetic uses checked operations; overflow must not cause
  silent wraparound or premature timeout.

### Mock time contract

- `MockClockControl::advance_clock(d)` advances virtual time by `d` and
  synchronously dispatches all timers that expire at or before the new
  time.
- `MockClockControl::reset_clock()` resets time to zero and clears the
  timer registry. Only for test initialization; must not be called
  while timers from a previous test are still alive.
- Mock `delay(d)` is equivalent to `advance_clock(d)` — it is
  deterministic and does not sleep.

---

## 7. Object lifecycle

All OSAL objects follow a common lifecycle:

```
Create ──→ (Start) ──→ Use ──→ Delete / Drop
```

### Creation

- Constructor functions accept configuration parameters (capacity,
  message size, max count, stack size, priority, etc.).
- Invalid parameters (zero capacity, count > max) return
  `Error::InvalidParameter`.
- Allocation failure returns `Error::OutOfMemory`.

### Usage

- Objects are usable immediately after creation (unless an explicit
  `start` step is documented).
- Operations on a deleted/dropped object have undefined behavior.
  Backends should make a best-effort attempt to fail safely, but this
  is not guaranteed.
- All public methods are thread-safe unless documented otherwise.

### Deletion

- `Drop` releases resources when no operation is actively borrowing
  the object. `Drop` must not block indefinitely.
- Explicit `close()` / `delete` operations (when provided) are
  responsible for waking blocked waiters before the underlying
  resource is released.
- Last-handle `Drop` is resource cleanup only; wake semantics are
  defined in `object-lifetime.md`.
- After deletion, the object's handle (if any) becomes invalid.

---

## 8. Task contract

### Type: `Task`

An independent execution context (thread / RTOS task).

### Creation

```rust
pub struct TaskBuilder { ... }

impl TaskBuilder {
    pub fn new() -> Self;
    pub fn name(self, name: &str) -> Self;
    pub fn stack_size(self, bytes: usize) -> Self;
    pub fn priority(self, prio: Priority) -> Self;
    pub fn spawn<F>(self, entry: F) -> Result<Task>
        where F: FnOnce() + Send + 'static;
}
```

### Builder rules

| Field        | Default     | Valid range |
|-------------|-------------|-------------|
| `name`      | `""` (empty) | 0..31 bytes, no embedded NUL; >31 bytes → `Error::InvalidParameter` |
| `stack_size` | `4096`      | >0; `0` → `Error::InvalidParameter`. Backend may round up to a platform minimum |
| `priority`  | `1`         | all `u32` values accepted; stored and returned as-is |

- `spawn` returns `Error::InvalidParameter` if the name exceeds 31
  bytes, contains embedded NUL, or `stack_size == 0`.
- `spawn` returns `Error::OutOfMemory` if the task cannot be
  allocated.
- The entry function `F` executes exactly once in the new task.
- After `spawn` returns `Ok`, the task may already be `Running` or
  even `Finished`; portable code must not assume it is in `Ready`.
- Calling `spawn` twice on the same builder is not possible (it
  consumes `self`).

### Lifecycle methods

```rust
impl Task {
    pub fn join(&self, timeout: Timeout) -> Result<ExitCode>;
    pub fn handle(&self) -> TaskHandle;
    pub fn priority(&self) -> Priority;
}
```

- `entry` return: the entry function (`FnOnce() + Send + 'static`)
  returns `()`; normal return maps to `ExitCode::SUCCESS`.
- `join(timeout)`: blocks until the task exits.
  - Returns `Ok(ExitCode)` on successful join.
  - Once the task has exited, the exit code is cached; subsequent
    calls to `join` return it immediately without blocking.
  - Returns `Error::Timeout` if the task does not exit within the
    timeout. The caller retains the handle and may retry.
- `drop`: dropping a `Task` handle does **not** cancel the task or
  kill the thread. The task continues to run independently.
- `handle()`: returns a non-zero `TaskHandle` uniquely identifying
  this task within the process.
- `priority()`: returns the task's configured priority. Priority is
  stored and reported; scheduling effect is backend-specific.

### Static methods

```rust
impl Task {
    pub fn current() -> Option<TaskHandle>;
    pub fn count() -> usize;
}
```

- `current()`: returns `Some(TaskHandle)` when called from within an
  OSAL-created task's entry function. Returns `None` from the main
  thread or any non-OSAL pthread.
- `count()`: returns the number of OSAL tasks whose entry function
  has not yet completed. Finished tasks whose handle still exists are
  **not** counted. The value is a snapshot for diagnostics only; do
  not use for concurrency synchronisation.

### Task state

| State | Meaning |
|-------|---------|
| `Ready` | Task created and eligible to run |
| `Running` | Task currently executing |
| `Blocked` | Task waiting on a synchronization primitive |
| `Suspended` | Task explicitly suspended (backend-dependent) |
| `Finished` | Task entry function returned |

- State transitions are backend-dependent. Portable code queries state
  for diagnostic purposes only — it must not use state to make
  correctness decisions.

### Exit codes

```rust
pub struct ExitCode(u32);

impl ExitCode {
    pub const SUCCESS: ExitCode = ExitCode(0);
    pub fn new(code: u32) -> Self;
    pub fn code(&self) -> u32;
}
```

---

## 9. Mutex contract

### Type: `Mutex<T>`

A **non-recursive** mutual exclusion lock protecting a value of type `T`.

### Creation

```rust
impl<T> Mutex<T> {
    pub fn new(value: T) -> Result<Self>;
}
```

- Allocates the mutex and stores `value`.
- Returns `Error::OutOfMemory` on allocation failure.

### Locking

```rust
impl<T> Mutex<T> {
    pub fn lock(&self, timeout: Timeout) -> Result<MutexGuard<T>>;
}
```

- `lock(timeout)`:
  - Acquires the mutex, blocking up to `timeout`.
  - On success, returns a `MutexGuard` that provides `&mut T` access
    via `DerefMut`.
  - Dropping the `MutexGuard` releases the lock.
  - **Non-recursive**: the owning task must not call `lock` again while
    a guard is alive. Attempting to do so returns `Error::LockFailed`
    (for `NoWait`).
  - Returns `Error::Timeout` if the timeout expires.
  - Returns `Error::LockFailed` if `Timeout::NoWait` and the mutex
    is held.
- Recursive locking is deferred to a future `RecursiveMutex` type.

> **ISR note:** Mutex operations are **not** ISR-safe. Use
> [`Semaphore`] or future ISR extension traits for interrupt-context
> signaling.

### MutexGuard

```rust
pub struct MutexGuard<'a, T> { ... }

impl<T> Deref for MutexGuard<'_, T> { type Target = T; ... }
impl<T> DerefMut for MutexGuard<'_, T> { ... }
impl<T> Drop for MutexGuard<'_, T> { /* releases the lock */ }
```

- `MutexGuard` is `!Send` (it represents ownership of a task-local
  lock).
- Dropping the guard when the mutex has been deleted has undefined
  behavior (the guard should not outlive the mutex).

### Timeout behavior

| Timeout | Behavior |
|---------|----------|
| `NoWait` | Return immediately. If held by another task → `Error::LockFailed`. |
| `After(d)` | Block until acquired or `d` elapsed. Zero duration that cannot immediately acquire → `Error::Timeout`. |
| `Forever` | Block until acquired. Must not return `Error::Timeout`. |

`After(Duration::ZERO)` is distinct from `NoWait`:
- `NoWait` on held mutex → `Error::LockFailed`
- `After(ZERO)` on held mutex → `Error::Timeout`

This is consistent with the Queue contract's timeout semantics.

### Error mapping

| Condition | Error |
|-----------|-------|
| Mutex held by another task, `NoWait` | `Error::LockFailed` |
| Timeout expired before acquisition | `Error::Timeout` |
| Allocation failure | `Error::OutOfMemory` |
| POSIX `EDEADLK` (re-entrant lock, same thread) | `Error::LockFailed` |

### Non-requirements

- **No ISR support**: Mutex operations are not ISR-safe.
- **No poisoning**: OSAL does not expose `std::sync::PoisonError`.
  Lock failures are platform errors, not data-corruption signals.
- **No fairness guarantee**: Backends are not required to implement
  fair (FIFO) wake ordering. Starvation is possible on some platforms.
- **No manual unlock**: Unlock is through Guard Drop only. There is no
  `unlock(&self)` method on the mutex.

### Deletion

- Dropping a `Mutex<T>` while locked: the behavior is backend-defined.
  On POSIX, the mutex is destroyed; on FreeRTOS, this is undefined.
  Portable code must ensure the mutex is unlocked before drop.

---

## 10. Semaphore contract

### Type: `CountingSemaphore`

A counting semaphore for resource management and task signaling.

### Creation

```rust
impl CountingSemaphore {
    pub fn new(max_count: u32, initial_count: u32) -> Result<Self>;
    pub fn max_count(&self) -> u32;
    pub fn count(&self) -> Result<u32>;
}
```

- `new(max, initial)`:
  - Returns `Error::InvalidParameter` if `initial > max` or `max == 0`.
  - Returns `Error::OutOfMemory` on allocation failure.
- `max_count()`: returns the configured maximum count (non-fallible; fixed at construction).
- `count()`: returns the current count. May fail if the backend cannot
  acquire the internal lock. The returned value is a snapshot; the
  actual count may change immediately after return.

### Operations

```rust
impl CountingSemaphore {
    pub fn acquire(&self, timeout: Timeout) -> Result<()>;
    pub fn release(&self) -> Result<()>;
}
```

- `acquire(timeout)`:
  - If `count > 0`: decrement and return `Ok(())`.
  - If `count == 0` and `NoWait`: return `Error::Timeout`.
  - If `count == 0` and `After(d)`: block until `release()` wakes us or
    timeout expires. `After(ZERO)` returns `Error::Timeout` if unavailable.
  - If `count == 0` and `Forever`: block until `release()` wakes us.
  - Each `release()` wakes at most one blocked acquirer.
- `release()`:
  - If `count < max_count`: increment and wake one acquirer.
  - If `count == max_count`: return `Error::Overflow` (the
    semaphore is already at maximum count). Count is unchanged.

> **ISR note:** ISR-safe acquire/release are deferred to a future
> `IsrSemaphore` extension trait (see ADR 0008). The core
> `CountingSemaphore` trait only provides task-context operations.

### Timeout behavior

| Timeout | `count > 0` | `count == 0` |
|---------|-------------|--------------|
| `NoWait` | Immediate success | `Error::Timeout` |
| `After(ZERO)` | Immediate success | `Error::Timeout` |
| `After(d>0)` | Immediate success | Block; `Error::Timeout` at deadline |
| `Forever` | Immediate success | Block until `release()` |

POSIX backends must use `CLOCK_MONOTONIC` for `After(d)` deadlines.

### Handle Clone

Semaphore handles may be cloned. All clones share the same underlying
count state. Dropping one clone does not affect the resource; the
resource is freed only when the last clone is dropped. (See ADR 0006.)

### Type: `BinarySemaphore`

A convenience wrapper around `CountingSemaphore` with `max_count = 1`,
initial count = 0 (unsignaled).

```rust
impl BinarySemaphore {
    pub fn new() -> Result<Self>;
    pub fn acquire(&self, timeout: Timeout) -> Result<()>;
    pub fn release(&self) -> Result<()>;
    pub fn is_signaled(&self) -> Result<bool>;
}
```

- `new()`: creates with `count = 0`, `max_count = 1` (unsignaled).
- `is_signaled()`: returns `Ok(true)` if `count == 1`, `Ok(false)` if
  `count == 0`. May fail if the internal lock cannot be acquired.
- `release()` when already signaled returns `Error::Overflow`.
- All methods delegate to the underlying `CountingSemaphore`.

---

## 11. Queue contract

### Type: `Queue`

A bounded FIFO message queue for inter-task byte-message communication.

### Creation

```rust
impl Queue {
    pub fn new(capacity: usize, msg_size: usize) -> Result<Self>;
    pub fn capacity(&self) -> usize;
    pub fn msg_size(&self) -> usize;
    pub fn len(&self) -> Result<usize>;
    pub fn is_empty(&self) -> Result<bool>;
    pub fn is_full(&self) -> Result<bool>;
}
```

- `new(capacity, msg_size)`:
  - Returns `Error::InvalidParameter` if `capacity == 0` or
    `msg_size == 0`.
  - Returns `Error::Overflow` if `capacity * msg_size` would overflow `usize`.
  - Returns `Error::OutOfMemory` on allocation failure.
- `capacity()`: maximum number of messages (non-fallible; fixed at construction).
- `msg_size()`: fixed size of each message in bytes (non-fallible; fixed at construction).
- `len()`: current number of messages in the queue. Returns `Result<usize>`
  because backends that synchronize internal state (e.g. via mutex) may
  encounter lock acquisition failures.
- `is_empty()` / `is_full()`: convenience queries; may fail if the
  underlying lock acquisition fails.

### Operations

```rust
impl Queue {
    pub fn send(&self, data: &[u8], timeout: Timeout) -> Result<()>;
    pub fn recv(&self, buffer: &mut [u8], timeout: Timeout) -> Result<()>;
    pub fn close(&self) -> Result<()>;
}
```

- `send(data, timeout)`:
  - `data.len()` must equal `msg_size()`; otherwise
    `Error::InvalidMessageSize`.
  - If not full: copy `data` into the queue, wake one blocked receiver,
    return `Ok(())`.
  - If full and `NoWait`: return `Error::QueueFull`.
  - If full and `After(d)`: block until space available or timeout.
  - If full and `Forever`: block until space available.
  - If the queue has been `close()`d: return `Error::QueueClosed`.
- `recv(buffer, timeout)`:
  - `buffer.len()` must equal `msg_size()`; otherwise
    `Error::InvalidMessageSize`.
  - If not empty: copy the oldest message into `buffer`, wake one
    blocked sender, return `Ok(())`.
  - If empty and `NoWait`: return `Error::QueueEmpty`.
  - If empty and `After(d)`: block until message available or timeout.
  - If empty and `Forever`: block until message available.
  - If the queue has been `close()`d and is empty: return
    `Error::QueueClosed`.
- `close()`:
  - Marks the queue as closed.
  - Wakes all blocked senders (they return `Error::QueueClosed`).
  - Wakes blocked receivers if the queue is empty.
  - If messages are already queued at close time, subsequent `recv`
    calls continue to drain them.
  - Does **not** discard already enqueued messages.
  - Idempotent: calling `close()` multiple times is safe.
  - Returns `Ok(())` on success.

> **ISR note:** ISR-safe send/receive (`send_from_isr`, `recv_from_isr`)
> are deferred to a future `IsrQueue` extension trait for the FreeRTOS
> backend. The core `Queue` trait only provides task-context operations.

**After-close rules:**

| Operation | Behavior after close |
|-----------|---------------------|
| `send` | Always returns `Error::QueueClosed` |
| `recv` | Succeeds while queued messages remain |
| `recv` | Returns `Error::QueueClosed` when closed **and** empty |
| `len` / `is_empty` / `is_full` / `capacity` / `msg_size` | Continue to work |

### FIFO guarantee

Messages are received in the order they were sent. If task A sends M1
then M2, and task B receives twice, B receives M1 then M2.

---

## 12. Timer contract

### Type: `Timer`

A software timer that invokes a callback after a specified period.
`Timer` requires `Clone` (per ADR 0006); all clones share the same
underlying timer.

### Creation

```rust
pub enum TimerMode {
    OneShot,
    Periodic,
}

pub type TimerCallback = Box<dyn FnMut() + Send + 'static>;

impl Timer {
    pub fn new(
        name: &str,
        period: Duration,
        mode: TimerMode,
        callback: TimerCallback,
    ) -> Result<Self>;
}
```

- `new(name, period, mode, callback)`:
  - Returns `Error::InvalidParameter` if `period` is zero.
  - Returns `Error::OutOfMemory` on allocation failure.
  - The timer is created in the **stopped** state.
  - The callback is invoked when the timer expires.

### Operations

```rust
impl Timer {
    pub fn start(&self) -> Result<()>;
    pub fn stop(&self) -> Result<()>;
    pub fn reset(&self) -> Result<()>;
    pub fn change_period(&self, new_period: Duration) -> Result<()>;
}
```

- `start()`:
  - Stopped → deadline = now + period, running = true.
  - Already running → equivalent to `reset()`.
- `stop()`:
  - Idempotent. Prevents future callbacks.
  - In-flight callbacks are not interrupted.
- `reset()`:
  - deadline = now + period, running = true.
  - Stopped → starts. Running → discards old deadline.
- `change_period(new_period)`:
  - Returns `Error::InvalidParameter` if `new_period` is zero.
  - Stopped → updates period; next start uses new period.
  - Running → updates period; current deadline unchanged; new period
    takes effect on the **next** expiration.

### Callback execution

- Callbacks execute in a **timer service context** (Mock: synchronously
  in `advance_clock`; POSIX: single background pthread). Callbacks are
  **not** invoked from ISR context.
- Callbacks execute **outside** the timer management lock; they may
  call other OSAL APIs including timer control operations.
- A single timer's callback is **never** called concurrently with
  itself (serial execution).
- **OneShot**: fires once, then transitions to stopped. If
  `start()`/`reset()` is called during callback execution, the new
  state takes precedence.
- **Periodic**: fires, then auto-reloads. The next deadline is computed
  as `previous_deadline + period` (fixed-rate), not from callback
  completion time. If one or more periods were missed, only one
  callback fires; the next deadline advances to the first multiple of
  `period` strictly after `now`.

### Generation and stale expiration

Every state change (`start`, `stop`, `reset`, `change_period`,
last-handle `drop`) increments a generation counter. When a callback
completes, if the generation has changed, the timer's state was
modified during callback execution and the old expiration logic
(auto-reload for Periodic) is skipped.

### Handle Clone and Drop

- `Clone` creates another handle to the same timer. All handles are
  equal; any handle can start, stop, reset, or change_period.
- Dropping one handle does not affect the timer. The timer is cancelled
  only when the **last** handle is dropped.
- Last-handle drop prevents future callbacks. In-flight callbacks are
  not waited for; they complete independently.
- The timer service registry must not hold strong references to timer
  handles (prevents leaks).

---

## 13. System contract

### Type: `System`

Global system-level operations. `System` provides portable heap
introspection and critical-section entry/exit.

Backend-specific scheduler control, ISR yield, interrupt masking
details, and board resource management are **not** part of the
portable `System` trait.

### Heap introspection

```rust
fn heap_free() -> usize;
```

Rules:

- `heap_free()` must be callable at any time.
- `heap_free()` must not panic.
- The returned value is a snapshot and may become stale immediately.
- If the backend can report free heap bytes, it should return that
  value.
- Host virtual-memory backends such as POSIX may return `usize::MAX`.
- Mock backends may return `usize::MAX`.

### Critical sections

```rust
type CriticalSectionGuard: Drop;

fn enter_critical() -> Self::CriticalSectionGuard;
```

Rules:

- `enter_critical()` enters one critical-section nesting level.
- Each call returns one guard.
- Dropping a guard exits one nesting level.
- Critical sections may be nested.
- The critical section is fully exited only after **all** nested
  guards have been dropped.
- Dropping a guard must not panic.
- Guard types must not be manually constructible outside the backend
  implementation.
- Critical sections are intended for short, infrequent operations
  only.

### Backend mapping

| Backend   | Critical section implementation | `heap_free()` |
|-----------|--------------------------------|---------------|
| Mock      | Atomic nesting counter         | `usize::MAX`  |
| POSIX     | Process-local recursive `pthread_mutex_t` | `usize::MAX` |
| FreeRTOS  | Deferred                       | Backend-defined |

### Non-requirements

The portable `System` trait does **not** require:

- scheduler start/stop
- interrupt enable/disable
- ISR yield
- heap region enumeration
- board memory-region reporting
- task lifecycle management
- real-time interrupt masking on host backends

### Contract tests

System contract tests must verify:

1. `heap_free()` is callable.
2. `enter_critical()` returns a guard.
3. Dropping the guard exits the critical section.
4. Nested critical sections are allowed.
5. Nested guards may be dropped in reverse order.

---

## 14. Unsupported capability rules

Some backends cannot implement every operation. The following rules
govern how unsupported capabilities must be handled:

1. **Return `Error::Unsupported`.** The operation must return this
   error consistently, not a different error or a panic.
2. **Document it.** Each backend's module-level documentation must list
   all capabilities returning `Error::Unsupported`.
3. **Contract tests must skip.** The conformance test harness provides
   a mechanism to skip tests for backends that declare a capability as
   unsupported.
4. **No silent success.** A backend must not return `Ok` for an
   operation it does not actually perform. Signal-without-waking is
   not acceptable.

### Known backend limitations

| Capability | POSIX | Mock | Future FreeRTOS |
|------------|-------|------|-----------------|
| ISR operations | Not supported | Not supported | True ISR (extension trait) |
| Task priority | Informational | Informational | Hardware priority |
| Task current() | TLS-based | TLS-based | Backend-defined |
| Stack watermark | Not tracked | Not tracked | Hardware tracked |
| Scheduler start/stop | No-op | Deferred | Hardware scheduler |
| Critical section | Recursive mutex | Atomic counter | Interrupt disable |
| Suspend/resume task | Not supported | Deferred | Supported |

---

## 15. Mock backend requirements

The mock backend (`osal-backend-mock`) is a fully in-memory,
deterministic implementation used for unit tests and contract
verification.

### Required capabilities

1. **Deterministic time**: A fake clock that advances only when
   explicitly instructed. No real time passes.
2. **Fault injection**: Every operation has a configurable fault
   trigger that causes a specified error. For example:
   - "next acquire fails with Timeout"
   - "send number 3 fails with QueueFull"
3. **Operation recording**: Every call (method, arguments, return
   value) is recorded in a history log. Tests assert on this log.
4. **All operations non-blocking by default**: Blocking is simulated
   by time advancement. `Timeout::Forever` blocks until the
   corresponding wake event occurs. Task blocking and concurrency
   are deferred to a future cooperative mock scheduler.

### Contract test integration

Every backend-independent core behavioral requirement must be testable
against Mock. Scheduler-dependent and true-concurrency requirements may
be verified only by backends that provide those capabilities (POSIX,
future FreeRTOS). Tests that pass on both Mock and POSIX are considered
validated.

---

## 16. POSIX backend requirements

The POSIX backend (`osal-backend-posix`) implements all OSAL primitives
using pthread and related POSIX APIs.

### Required primitives

| OSAL type | POSIX implementation |
|-----------|---------------------|
| Mutex | `pthread_mutex_t` (PTHREAD_MUTEX_ERRORCHECK) |
| CountingSemaphore | `pthread_mutex_t` + `pthread_cond_t` + count variable |
| Queue | `pthread_mutex_t` + two `pthread_cond_t` (not_empty, not_full) + ring buffer |
| Task | `pthread_create` / `pthread_join` |
| Timer | `pthread_cond_timedwait` with CLOCK_MONOTONIC in a background worker thread |
| Clock | `clock_gettime(CLOCK_MONOTONIC)` |
| Critical section | Process-local recursive `pthread_mutex_t` with per-thread nesting via `pthread_key_t` TLS |

### Specific requirements

1. **Monotonic clock**: All time operations use `CLOCK_MONOTONIC`.
   Wall-clock changes must not affect OSAL timing.
2. **Thread-safe initialization**: Global state (clock epoch, registry)
   uses `pthread_once_t`.
3. **No ISR support**: POSIX does not implement ISR-safe operations.
   The core `Queue` and `Mutex` traits do not include ISR methods.
   ISR operations are deferred to extension traits for the FreeRTOS
   backend.
4. **Scheduler no-ops**: `System::start()` and `System::stop()` are
   documented no-ops. Tasks run when created.
5. **Priority is informational**: Task priority maps to pthread
   scheduling policy attributes only if real-time scheduling is
   explicitly enabled.
6. **Heap reporting**: `heap_free()` returns `usize::MAX` (host virtual
   memory).
7. **Task identity**: `current()` returns `Some(TaskHandle)` inside
   an OSAL-created task via `thread_local!` TLS. `count()` returns
   the number of tasks whose entry function has not yet completed.
   A full task registry is deferred (see ADR 0013).

---

## 17. Conformance test matrix

Each behavioral requirement maps to one or more contract tests.
Backends must pass all non-skipped tests.

### Legend

- **R**: Required — all backends must pass
- **P**: POSIX only — requires host OS features
- **M**: Mock only — tests fault injection or deterministic behavior
- **S**: Skipped — backend declares this capability unsupported

### Task tests — Core (Mock + POSIX)

| Test | Requirement | POSIX | Mock |
|------|-------------|-------|------|
| Create with default config | Builder defaults compile and spawn | R | R |
| Accept empty name | `""` is valid | R | R |
| Reject name > 31 bytes | `Error::InvalidParameter` | R | R |
| Reject embedded NUL | `Error::InvalidParameter` | R | R |
| Reject zero stack size | `Error::InvalidParameter` | R | R |
| Spawn and join successfully | Task runs, join returns ExitCode | R | R |
| Repeated join returns cached | Subsequent joins return immediately | R | R |
| Handle is non-zero | `TaskHandle` is unique and non-zero | R | R |
| current() from within task | Returns `Some(TaskHandle)` | R | R |
| current() from main thread | Returns `None` | R | R |
| count() reflects live tasks | Entry running → count ≥ 1; returned → count back to baseline | R | R |
| Priority preserved | `priority()` returns configured value | R | R |

### Task tests — Concurrency (POSIX only)

| Test | Requirement | POSIX | Mock |
|------|-------------|-------|------|
| Multiple concurrent tasks | 3+ tasks run simultaneously | R | S |
| Join with timeout | `Error::Timeout` on non-exiting task | R | S |
| Timeout then retry | Timeout → retry Forever succeeds | R | S |
| Drop without join | Task continues running after handle drop | R | S |

### Task tests — Deferred (neither)

| Test | Requirement | POSIX | Mock |
|------|-------------|-------|------|
| Suspend / resume | Task pauses and resumes | S | S |
| Cancel / kill | Forced task termination | S | S |
| Deterministic mock scheduling | Cooperative yield, virtual-time scheduling | S | S |

### Mutex tests

| Test | Requirement | POSIX | Mock |
|------|-------------|-------|------|
| Create and store value | `Mutex::new(v)` works | R | R |
| Lock and unlock | Guard provides &mut T, drop releases | R | R |
| Non-recursive: second lock fails | Re-lock while held → LockFailed | R | R |
| Cross-task mutual exclusion | Other task blocks while locked | R | R |
| Non-blocking try-lock | `Timeout::NoWait` returns `LockFailed` if held | R | R |
| Timeout expires | `Timeout::After(d)` returns `Timeout` | R | R |
| Forever blocks until release | `Timeout::Forever` succeeds after release | R | R |
| Guard is `!Send` | Compile-time check | R | R |

### Semaphore tests

| Test | Requirement | POSIX | Mock |
|------|-------------|-------|------|
| Create with valid counts | `new(max, initial)` works | R | R |
| Reject initial > max | `Error::InvalidParameter` | R | R |
| Reject max == 0 | `Error::InvalidParameter` | R | R |
| acquire decrements count | count goes from N to N-1 | R | R |
| release increments count | count goes from N to N+1 | R | R |
| acquire blocks on empty | Task waits until release | R | S |
| Timeout on empty | `Timeout::After(d)` returns `Timeout` | R | R |
| release at max fails | `Error::Overflow` | R | R |
| release wakes exactly one | N releases wake N waiters, not more | R | S |
| BinarySemaphore basics | `new()`, `acquire()`, `release()`, `is_signaled()` | R | R |
| Clone shares state | Clone sees same count | R | R |

### Queue tests

| Test | Requirement | POSIX | Mock |
|------|-------------|-------|------|
| Create with valid params | `Queue::new(cap, size)` works | R | R |
| Reject zero capacity | `Error::InvalidParameter` | R | R |
| Reject zero msg_size | `Error::InvalidParameter` | R | R |
| Send and recv single message | Round-trip preserves bytes | R | R |
| FIFO ordering | Messages received in send order | R | R |
| Send blocks on full | Sender waits until recv | R | R |
| Recv blocks on empty | Receiver waits until send | R | R |
| Non-blocking send on full | `Error::QueueFull` | R | R |
| Non-blocking recv on empty | `Error::QueueEmpty` | R | R |
| Message size mismatch | `Error::InvalidMessageSize` on send/recv | R | R |
| Close wakes blocked senders | Pending sends return `QueueClosed` | R | R |
| Close wakes receivers only if empty | Blocked receivers on empty queue return `QueueClosed` | R | R |
| Close is idempotent | Calling close twice is safe | R | R |
| Send fails after close | `send` returns `QueueClosed` | R | R |
| Recv drains remaining after close | `recv` succeeds while messages remain | R | R |
| Recv fails after close and empty | `recv` returns `QueueClosed` | R | R |

### Timer tests

| Test | Requirement | POSIX | Mock |
|------|-------------|-------|------|
| Create one-shot timer | `Timer::new(... OneShot)` succeeds | R | R |
| Create periodic timer | `Timer::new(... Periodic)` succeeds | R | R |
| Reject zero period | `Error::InvalidParameter` | R | R |
| One-shot fires once | Callback invoked exactly once | R | R |
| Periodic fires multiple times | Callback invoked >= 2 times | R | R |
| Stop prevents callback | Stopped timer does not fire | R | R |
| Reset restarts countdown | Timer fires period after reset | R | R |
| Change period updates timing | New period takes effect | R | R |
| Callback outside lock | Nested timer operations in callback OK | R | R |

### Clock and System tests

| Test | Requirement | POSIX | Mock |
|------|-------------|-------|------|
| now() is monotonic | `now()` never decreases | R | R |
| elapsed() is correct | `elapsed(s) + s ≈ now()` | R | R |
| delay() blocks at least d | Tick count increased after delay | R | R |
| delay(0) returns immediately | Zero delay is near-instant | R | R |
| heap_free() returns value | Non-zero on POSIX, usize::MAX OK | R | R |
| task_count() returns tasks | Matches spawned count | R | R |
| Critical section mutual exclusion | Nested enter/exit are safe | R | R |

---

---

> For object ownership, lifetime, clone semantics, and destruction
> safety rules, see [Object Lifetime and Ownership](object-lifetime.md).
