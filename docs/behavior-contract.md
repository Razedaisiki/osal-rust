# OSAL Backend Contract

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
osal = { version = "0.1", features = ["mock"] }  # Mock
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
| `LockFailed` | Could not acquire lock | Mutex held by another context |
| `NotFound` | Resource not found | Invalid handle or ID |
| `InvalidParameter` | Argument out of valid range | Zero-length name, count > max |
| `AlreadyInitialized` | Resource already created/started | Double `spawn()` on a Task |
| `NotInitialized` | Resource not yet started | `join()` on unstarted Task |
| `Unsupported` | Backend cannot perform operation | ISR mutex on FreeRTOS |
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

### Clock contract

- `Clock::now()` returns a monotonically increasing `Duration` from an
  arbitrary epoch (typically process start or system boot).
- The clock must never jump backward.
- Resolution is backend-dependent; portable code must not assume
  sub-millisecond precision.
- `Clock::elapsed(since: Duration) -> Duration` is equivalent to
  `now() - since`, saturating at zero.

### Delay contract

- `Clock::delay(d: Duration)` blocks the calling task for **at least**
  `d`. It may block longer due to scheduling.
- `delay(Duration::ZERO)` must return immediately.
- Implementations should use the most efficient blocking primitive
  available (e.g. `nanosleep`, `pthread_cond_timedwait`, RTOS tick
  delay).

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

- `Drop` (or an explicit `delete`/`close` method if asynchronous
  cleanup is needed) releases all resources.
- Tasks blocked on a deleted object must be woken with an error.
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

| Field | Default | Valid range |
|-------|---------|-------------|
| `name` | `""` (empty) | 0..31 bytes, no embedded NUL |
| `stack_size` | `4096` | Minimum backend-defined (typically 512) |
| `priority` | `1` | 0..(backend max - 1) |

- `spawn` returns `Error::InvalidParameter` if any field is out of
  range.
- `spawn` returns `Error::OutOfMemory` if the task cannot be allocated.
- The entry function `F` executes exactly once in the new task.
- After `spawn` returns `Ok`, the task is in the `Ready` state.
- Calling `spawn` twice on the same builder is not possible (it
  consumes `self`).

### Lifecycle methods

```rust
impl Task {
    pub fn join(self, timeout: Timeout) -> Result<ExitCode>;
    pub fn handle(&self) -> Handle;
    pub fn priority(&self) -> Priority;
}
```

- `join(timeout)`: blocks until the task exits.
  - Returns `Ok(ExitCode)` on successful join.
  - Returns `Error::Timeout` if the task does not exit within the
    timeout.
  - Returns `Error::NotInitialized` if the task was never spawned.
  - After `join` returns `Ok`, the task handle is invalid and the
    `Task` is consumed.
- `handle()`: returns an opaque `Handle` uniquely identifying this
  task.
- `priority()`: returns the task's current priority.

### Static methods

```rust
impl Task {
    pub fn current() -> Handle;
    pub fn count() -> usize;
}
```

- `current()`: returns the handle of the calling task. Must work from
  any OSAL task context.
- `count()`: returns the number of tasks currently known to the
  system. Includes running, ready, blocked, and suspended tasks.

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

A recursive mutual exclusion lock protecting a value of type `T`.

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
    pub fn isr_lock(&self) -> Result<MutexGuard<T>>;
}
```

- `lock(timeout)`:
  - Acquires the mutex, blocking up to `timeout`.
  - On success, returns a `MutexGuard` that provides `&mut T` access
    via `DerefMut`.
  - Dropping the `MutexGuard` releases one level of the lock.
  - Recursive: the owning task may call `lock` again without blocking.
    Each `lock` must be matched by a corresponding guard drop.
  - Returns `Error::Timeout` if the timeout expires.
  - Returns `Error::LockFailed` if `Timeout::NoWait` and the mutex
    is held by another task.
- `isr_lock()`:
  - Non-blocking: returns immediately.
  - On platforms without true ISR context (POSIX, Mock), equivalent to
    `lock(Timeout::NoWait)`.
  - On platforms where ISR mutex operations are unsupported, returns
    `Error::Unsupported`.

### MutexGuard

```rust
pub struct MutexGuard<'a, T> { ... }

impl<T> Deref for MutexGuard<'_, T> { type Target = T; ... }
impl<T> DerefMut for MutexGuard<'_, T> { ... }
impl<T> Drop for MutexGuard<'_, T> { /* releases one lock level */ }
```

- `MutexGuard` is `!Send` (it represents ownership of a task-local
  lock).
- Dropping the guard when the mutex has been deleted has undefined
  behavior (the guard should not outlive the mutex).

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
    pub fn count(&self) -> u32;
}
```

- `new(max, initial)`:
  - Returns `Error::InvalidParameter` if `initial > max` or `max == 0`.
  - Returns `Error::OutOfMemory` on allocation failure.
- `max_count()`: returns the configured maximum count.
- `count()`: returns the current count (snapshot; may change
  immediately after return).

### Operations

```rust
impl CountingSemaphore {
    pub fn acquire(&self, timeout: Timeout) -> Result<()>;
    pub fn release(&self) -> Result<()>;

    pub fn isr_acquire(&self) -> Result<()>;
    pub fn isr_release(&self) -> Result<()>;
}
```

- `acquire(timeout)`:
  - If `count > 0`: decrement and return `Ok(())`.
  - If `count == 0` and `NoWait`: return `Error::Timeout`.
  - If `count == 0` and `After(d)`: block until `release()` wakes us or
    timeout expires.
  - If `count == 0` and `Forever`: block until `release()` wakes us.
  - Wakes exactly one blocked acquirer per `release()`.
- `release()`:
  - If `count < max_count`: increment and wake one acquirer.
  - If `count == max_count`: return `Error::InvalidParameter` (the
    semaphore is full).
- `isr_acquire()`: non-blocking; equivalent to
  `acquire(Timeout::NoWait)`.
- `isr_release()`: ISR-safe; may be called from interrupt context.

### Type: `BinarySemaphore`

A convenience wrapper around `CountingSemaphore` with `max_count = 1`.

```rust
impl BinarySemaphore {
    pub fn new() -> Result<Self>;
    pub fn acquire(&self, timeout: Timeout) -> Result<()>;
    pub fn release(&self) -> Result<()>;
    pub fn is_acquired(&self) -> bool;
    pub fn isr_acquire(&self) -> Result<()>;
    pub fn isr_release(&self) -> Result<()>;
}
```

- `new()`: creates with `count = 0`, `max_count = 1`.
- `is_acquired()`: returns `true` if `count == 1`.
- All other methods delegate to the underlying `CountingSemaphore`.

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
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn is_full(&self) -> bool;
}
```

- `new(capacity, msg_size)`:
  - Returns `Error::InvalidParameter` if `capacity == 0` or
    `msg_size == 0`.
  - Returns `Error::OutOfMemory` on allocation failure.
- `capacity()`: maximum number of messages.
- `msg_size()`: fixed size of each message in bytes.
- `len()`: current number of messages in the queue.
- `is_empty()` / `is_full()`: convenience queries.

### Operations

```rust
impl Queue {
    pub fn send(&self, data: &[u8], timeout: Timeout) -> Result<()>;
    pub fn recv(&self, buffer: &mut [u8], timeout: Timeout) -> Result<()>;
    pub fn close(&self);

    pub fn isr_send(&self, data: &[u8]) -> Result<()>;
    pub fn isr_recv(&self, buffer: &mut [u8]) -> Result<()>;
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
  - Wakes all blocked senders and receivers (they return
    `Error::QueueClosed`).
  - Subsequent operations return `Error::QueueClosed`.
  - Idempotent: calling `close()` multiple times is safe.
- `isr_send(data)`: non-blocking; equivalent to
  `send(data, Timeout::NoWait)`.
- `isr_recv(buffer)`: non-blocking; equivalent to
  `recv(buffer, Timeout::NoWait)`.

### FIFO guarantee

Messages are received in the order they were sent. If task A sends M1
then M2, and task B receives twice, B receives M1 then M2.

---

## 12. Timer contract

### Type: `Timer`

A software timer that invokes a callback after a specified period.

### Creation

```rust
pub enum TimerMode {
    OneShot,
    Periodic,
}

pub type TimerCallback = Box<dyn Fn() + Send + 'static>;

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
  - The callback is invoked when the timer expires. It must not panic
    (panic in callback aborts the process on `panic=abort`).

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
  - Begins the countdown. If already running, behaves like `reset()`.
  - The callback fires after `period` has elapsed.
- `stop()`:
  - Prevents future callbacks. In-flight callbacks are not interrupted.
  - If already stopped, this is a no-op.
- `reset()`:
  - Restarts the countdown from now. If stopped, also starts the timer.
- `change_period(new_period)`:
  - Updates the period. Takes effect on the next expiration.
  - Returns `Error::InvalidParameter` if `new_period` is zero.

### Callback execution

- **OneShot**: the callback fires once, then the timer stops.
- **Periodic**: the callback fires, then the timer automatically
  reloads. The next countdown begins from the scheduled expiration
  time (not from callback completion) where practical.
- Callbacks execute outside the timer management lock.
- Callbacks execute in a timer service context (not ISR).
- Callbacks should be short and non-blocking.

### Deletion

- Dropping a `Timer` stops it and frees resources. In-flight callbacks
  are not interrupted.

---

## 13. Unsupported capability rules

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
| ISR operations | Try-lock, non-blocking | Try-lock, non-blocking | True ISR |
| Task priority | Informational | Deterministic order | Hardware priority |
| Stack watermark | Not tracked | Not tracked | Hardware tracked |
| Scheduler start/stop | No-op | Controllable | Hardware scheduler |
| Critical section | Recursive mutex | Recursive mutex | Interrupt disable |
| Suspend/resume task | Not supported | Supported | Supported |

---

## 14. Mock backend requirements

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
   corresponding wake event occurs.
5. **Deterministic task ordering**: Tasks run in priority order; at
   equal priority, the first spawned runs first. Context switches
   occur at explicit yield points.

### Contract test integration

The mock backend is the primary target for contract tests. Every
behavioral requirement in this document must be testable against the
mock backend. Tests that pass on mock and POSIX are considered
validated.

---

## 15. POSIX backend requirements

The POSIX backend (`osal-backend-posix`) implements all OSAL primitives
using pthread and related POSIX APIs.

### Required primitives

| OSAL type | POSIX implementation |
|-----------|---------------------|
| Mutex | `pthread_mutex_t` (PTHREAD_MUTEX_RECURSIVE for raw, PTHREAD_MUTEX_ERRORCHECK for `Mutex<T>`) |
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
3. **No real ISR**: `isr_*` methods are non-blocking try-operations.
   They must never block.
4. **Scheduler no-ops**: `System::start()` and `System::stop()` are
   documented no-ops. Tasks run when created.
5. **Priority is informational**: Task priority maps to pthread
   scheduling policy attributes only if real-time scheduling is
   explicitly enabled.
6. **Heap reporting**: `heap_free()` returns `usize::MAX` (host virtual
   memory).
7. **Cooperative cancellation**: Task deletion requests cancellation;
   tasks must periodically check and exit.
8. **Thread registration**: A registry tracks all OSAL tasks for
   introspection (`count()`, `current()`).

---

## 16. Conformance test matrix

Each behavioral requirement maps to one or more contract tests.
Backends must pass all non-skipped tests.

### Legend

- **R**: Required — all backends must pass
- **P**: POSIX only — requires host OS features
- **M**: Mock only — tests fault injection or deterministic behavior
- **S**: Skipped — backend declares this capability unsupported

### Task tests

| Test | Requirement | POSIX | Mock |
|------|-------------|-------|------|
| Create with default config | Builder defaults compile and spawn | R | R |
| Create with all fields set | name, stack, priority propagated | R | R |
| Reject zero-length name | `Error::InvalidParameter` | R | R |
| Reject zero stack size | `Error::InvalidParameter` | R | R |
| Spawn and join successfully | Task runs, join returns ExitCode | R | R |
| Join with timeout | `Error::Timeout` on non-exiting task | R | R |
| Join unstarted task | `Error::NotInitialized` | R | R |
| Multiple concurrent tasks | 3+ tasks run simultaneously | R | R |
| current() from within task | Returns correct handle | R | R |
| count() reflects reality | Matches number of spawned tasks | R | R |
| Suspend / resume | Task pauses and resumes | S | R |

### Mutex tests

| Test | Requirement | POSIX | Mock |
|------|-------------|-------|------|
| Create and store value | `Mutex::new(v)` works | R | R |
| Lock and unlock | Guard provides &mut T, drop releases | R | R |
| Recursive lock | Same task locks N times, unlocks N times | R | R |
| Cross-task mutual exclusion | Other task blocks while locked | R | R |
| Non-blocking try-lock | `Timeout::NoWait` returns `LockFailed` if held | R | R |
| Timeout expires | `Timeout::After(d)` returns `Timeout` | R | R |
| Forever blocks until release | `Timeout::Forever` succeeds after release | R | R |
| Guard is `!Send` | Compile-time check | R | R |
| isr_lock is non-blocking | Returns immediately | R | R |

### Semaphore tests

| Test | Requirement | POSIX | Mock |
|------|-------------|-------|------|
| Create with valid counts | `new(max, initial)` works | R | R |
| Reject initial > max | `Error::InvalidParameter` | R | R |
| Reject max == 0 | `Error::InvalidParameter` | R | R |
| acquire decrements count | count goes from N to N-1 | R | R |
| release increments count | count goes from N to N+1 | R | R |
| acquire blocks on empty | Task waits until release | R | R |
| Timeout on empty | `Timeout::After(d)` returns `Timeout` | R | R |
| release at max fails | `Error::InvalidParameter` | R | R |
| release wakes exactly one | N releases wake N waiters, not more | R | R |
| BinarySemaphore basics | `new()`, `acquire()`, `release()` | R | R |
| isr_acquire non-blocking | Returns immediately | R | R |
| isr_release from ISR context | Mock simulates ISR context | R | R |

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
| Close wakes blocked receivers | Pending recvs return `QueueClosed` | R | R |
| Close is idempotent | Calling close twice is safe | R | R |
| Operations after close | All return `QueueClosed` | R | R |

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
