# ADR 0027: FreeRTOS Queue Object Model

## Status

Accepted (2026-07-27)

## Context

The OSAL Queue trait requires bounded FIFO byte-message queues with
blocking send/recv, close-drain semantics, and clone-based handle
sharing. FreeRTOS provides native `xQueueCreate`/`xQueueSend`/
`xQueueReceive` with tick-based timeout, but native queues lack a
terminal close state that wakes all blocked senders and receivers
while allowing remaining messages to drain.

The `ByteQueue` portable ring buffer already implements the full OSAL
data/close state machine and is used by both the Mock and POSIX
backends. P7C delivered native FreeRTOS mutex and counting semaphore
wrappers, a unified wait engine (`wait.rs`), and a host fixture
capable of deterministic cross-thread waiter tests.

This ADR defines how the FreeRTOS backend implements Queue by composing
`ByteQueue` (data + close state) with native FreeRTOS mutex (state
protection) and counting semaphores (waiter wake signalling), avoiding
any dependency on native FreeRTOS queue primitives.

## Decision

### 1. Architecture: ByteQueue + native sync

```
FreeRtosQueue
    └── Arc<QueueInner>
            ├── RuntimeLease
            ├── native mutex (state_mutex)
            ├── native counting semaphore (sender_wake)
            ├── native counting semaphore (receiver_wake)
            └── UnsafeCell<QueueState>
                    ├── ByteQueue
                    ├── sender_waiters: u32
                    ├── receiver_waiters: u32
                    ├── sender_wake_credits: u32
                    └── receiver_wake_credits: u32
```

`ByteQueue` is the **sole source of truth** for message data and close state.
Native FreeRTOS queue primitives (`xQueueSend`/`xQueueReceive`) are NOT used.

The native mutex serialises access to `QueueState`. The two counting
semaphores serve only as conditional-wake channels — their counts are
NOT interpreted as queue length or condition truth. Every wake must
re-check `ByteQueue` state under the mutex.

### 2. Waiter-credit protocol

A naive `if waiters > 0 { give(sem) }` accumulates stale tokens: two
consecutive sends with one receiver waiting would deposit two tokens,
leaving the second as a spurious wake for a future receiver. Repeated
occurrences could overflow the semaphore count.

Each direction (sender, receiver) therefore maintains two counters:

```
waiters:  tasks that have registered and not yet exited the wait protocol
credits:  tokens already posted to the wake semaphore but not yet
          consumed (confirmed) by a waiter under the state mutex
```

**Core invariant** (per direction):

```
0 <= wake_credits <= waiters
```

**Normal wake-one** (e.g. receiver completing a recv, signalling a sender):

```rust
if sender_waiters > sender_wake_credits {
    semaphore_give(sender_wake);
    sender_wake_credits += 1;
}
```

At most one unconfirmed token exists per registered waiter.

**Close broadcast**:

```rust
let missing = waiters - wake_credits;
for _ in 0..missing {
    semaphore_give(wake_sem);
}
wake_credits = waiters;
```

Every registered waiter receives exactly one token. Already-posted
but unconfirmed tokens are not duplicated.

### 3. Lock order

```
1. Acquire state_mutex (native FreeRTOS mutex)
2. Inspect or mutate QueueState (ByteQueue + waiter/credit counters)
3. Optionally register as waiter
4. Release state_mutex
5. Block on sender_wake or receiver_wake semaphore
6. Re-acquire state_mutex
7. Confirm credit, unregister waiter, re-check QueueState
```

**MUST NOT block on a wake semaphore while holding the state mutex.**
Doing so would deadlock: a receiver holds the mutex waiting for a
message, while the sender cannot acquire the mutex to deliver one.

Non-blocking `semaphore_give()` while holding the mutex is permitted —
the woken task will briefly contend on the mutex, which is acceptable.

### 4. Internal mutex acquisition

Queue timeout controls "wait for space/message", not "wait for the
internal state mutex". The internal mutex should be acquired with an
optimistic zero-tick attempt first:

```rust
fn lock_state(&self) -> Result<QueueStateGuard<'_>> {
    match sys::mutex_take(state_mutex, 0) {
        Acquired => Ok(guard),
        Timeout => {
            // Contention — fall back to blocking.
            wait_native(Forever, |ticks| sys::mutex_take(state_mutex, ticks))?;
            Ok(guard)
        }
        Invalid => panic!("state mutex invalid on live queue"),
    }
}
```

This allows `NoWait` operations to succeed before the scheduler starts,
as long as there is no concurrent access.

`QueueStateGuard` is `!Send + !Sync` and releases the native mutex on Drop.

### 5. WaitBudget: single absolute deadline across repeated waits

The existing `wait_native(timeout, take)` in P7C is designed for
single-acquisition operations (Mutex, Semaphore). Queue operations may
require multiple wait attempts within one API call: a waiter wakes,
reacquires the mutex, discovers the condition has changed (another
waiter consumed the message, or the queue was closed), and must wait
again without resetting the original deadline.

Introduce `WaitBudget`:

```rust
pub(crate) enum WaitBudget {
    NoWait,
    Zero,
    Finite { duration: Duration, deadline: Option<Duration> },
    Forever,
}
```

- `NoWait` and `Zero`: never enter a blocking wait.
- `Finite`: the absolute `deadline` is computed **lazily** on the first
  call to `wait_once()`. If the operation succeeds immediately (queue
  has space/message on first check), no deadline is ever computed,
  avoiding spurious `Overflow` from `checked_add` on very large durations.
- `Forever`: loops max-finite chunks (same as P7C ADR 0025 §3).

Each `wait_once()` call consumes part of the budget. `Finite` returns
`Unavailable` when the deadline passes. `Forever` never returns
`Unavailable`.

The existing `wait_native()` is preserved as a convenience wrapper:

```rust
pub fn wait_native(timeout, take) -> Result<WaitOutcome> {
    let mut budget = WaitBudget::new(timeout);
    budget.wait_once(take)
}
```

P7C Mutex and Semaphore behaviour is unchanged.

### 6. Error precedence

Error precedence follows ADR 0001 and the existing Queue contract:

| Priority | Condition | Error |
|----------|-----------|-------|
| Highest | Wrong `data.len()` or `buffer.len()` | `InvalidMessageSize` |
| 2 | Queue closed (send) | `QueueClosed` |
| 3 | Queue full + `NoWait` | `QueueFull` |
| 3 | Queue empty + `NoWait` | `QueueEmpty` |
| 4 | Queue closed + empty (recv) | `QueueClosed` |
| 5 | Timeout expired | `Timeout` |

`InvalidMessageSize` takes priority over `QueueClosed` — a wrong-sized
send to a closed queue still returns `InvalidMessageSize`.

`After(ZERO)` returns `Timeout` (not `QueueFull`/`QueueEmpty`) when the
operation cannot complete immediately. `NoWait` returns the specific
error. This matches ADR 0025 §5.

When a timeout and close race, `QueueClosed` takes priority over
`Timeout`. The caller must not receive `Timeout` from a queue that is
observably closed.

### 7. Constructor and resource rollback

Constructor order:

```
1. Validate capacity, msg_size (including capacity * msg_size overflow)
2. Create ByteQueue (Rust allocation — fail early)
3. Acquire RuntimeLease
4. Create native state mutex
5. Create sender wake semaphore (counting, max = capacity, initial = 0)
6. Create receiver wake semaphore (counting, max = capacity, initial = 0)
7. Construct Arc<QueueInner>
```

Parameter validation and `ByteQueue` allocation precede any native
resource creation, so parameter errors and Rust OOM do not require
native-object rollback. A local RAII guard cleans up native objects
on construction failure (the plan's `NativeQueueResources` pattern).

### 8. Clone and Drop

`FreeRtosQueue` holds `Arc<QueueInner>`. `Clone` increments the Arc
strong count — no native handle duplication.

`QueueInner::drop()` runs when the last Arc reference is released:

```
1. Assert no registered waiters (defensive invariant check)
2. Delete receiver wake semaphore
3. Delete sender wake semaphore
4. Delete state mutex
5. Drop ByteQueue
6. Drop RuntimeLease
```

The application must ensure no `send()`/`recv()` call is in flight
when the last handle drops. The Rust type system cannot prevent this
(the `Arc` strong count reaches zero independently of outstanding
borrows). The backend module documentation MUST state this constraint.

### 9. Send + Sync

`QueueInner` is `Send + Sync` — FreeRTOS handles may be used from any
task context, and `ByteQueue` is accessed only under the native mutex.

`QueueStateGuard` (the mutex guard wrapper) is `!Send + !Sync` via
`PhantomData<Rc<()>>`, matching the Mutex guard pattern (ADR 0026 §4).

### 10. Scheduler-state behaviour

`NoWait` and `After(ZERO)` do NOT require the scheduler to be running
(they use zero-tick native operations which are non-blocking).

`After(d > 0)` and `Forever` require `SchedulerState::Running`.
`NotStarted` → `Error::NotInitialized`; `Suspended` → `Error::Busy`;
`Unknown` → `Error::Internal`. This matches ADR 0025 §4, enforced by
`WaitBudget::wait_once()` calling `ensure_blocking_allowed()` on first
blocking entry.

### 11. Close broadcast and wake failure policy

`close()` must attempt to wake ALL registered waiters in both directions.
After `ByteQueue::close()`:

```rust
// Wake all registered senders
let sender_missing = state.sender_waiters - state.sender_wake_credits;
for _ in 0..sender_missing {
    semaphore_give_or_fatal(sender_wake);
}
state.sender_wake_credits = state.sender_waiters;

// Wake all registered receivers
let receiver_missing = state.receiver_waiters - state.receiver_wake_credits;
for _ in 0..receiver_missing {
    semaphore_give_or_fatal(receiver_wake);
}
state.receiver_wake_credits = state.receiver_waiters;
```

If any `semaphore_give()` fails (`Full` or `Invalid`), the failure is
treated as a fatal invariant violation (panic). The caller must not
receive a recoverable error that leaves them unsure whether close was
committed — `ByteQueue::close()` has already executed. This policy is
inherited from POSIX P6D.

### 12. `send()` algorithm (blocking)

```
1. Validate data.len() == msg_size (error: InvalidMessageSize)
2. Acquire state_mutex
3. If ByteQueue is closed → QueueClosed
4. Try ByteQueue::try_send(data)
   - Ok(()) → signal_one_receiver_if_needed(), return Ok(())
   - QueueFull + NoWait → return QueueFull
   - QueueFull + Zero → return Timeout
   - QueueFull + (Finite|Forever) → register sender waiter, proceed
5. Drop state_mutex
6. WaitBudget::wait_once() on sender_wake
7. Re-acquire state_mutex
8. If Acquired → confirm sender credit, unregister waiter
9. Else (Unavailable/timeout) → race-reconcile: try take(0) on sender_wake
   - If token found → it was a close/wake race → confirm credit
   - If no token → unregister, check closed → QueueClosed or Timeout
10. Loop to step 4
```

### 13. `recv()` algorithm (blocking)

Symmetric to `send()`:

```
1. Validate buffer.len() == msg_size (error: InvalidMessageSize)
2. Acquire state_mutex
3. Try ByteQueue::try_recv(buffer)
   - Ok(n) → signal_one_sender_if_needed(), return Ok(())
   - QueueEmpty + closed → return QueueClosed
   - QueueEmpty + NoWait → return QueueEmpty
   - QueueEmpty + Zero → return Timeout
   - QueueEmpty + (Finite|Forever) → register receiver waiter, proceed
4. Drop state_mutex
5. WaitBudget::wait_once() on receiver_wake
6. Re-acquire state_mutex
7. If Acquired → confirm receiver credit, unregister waiter
8. Else → race-reconcile, re-check close/empty
9. Loop to step 3
```

### 14. ISR deferral

ISR-safe queue operations (`send_from_isr`, `recv_from_isr`) are
deferred per ADR 0003 and ADR 0008. The FreeRTOS backend does not
implement `IsrQueue` in P7D. Native FreeRTOS queue handles
(`QueueHandle`) in the `-sys` crate remain reserved for future ISR work.

## Consequences

- Queue semantics (FIFO, close-drain, error precedence) are guaranteed
  by `ByteQueue`, consistent with Mock and POSIX backends.
- Waiter-credit protocol prevents stale-token accumulation and wake
  semaphore overflow.
- Single absolute deadline across repeated waits prevents timeout
  creep from spurious wakeups.
- No dependency on native FreeRTOS queue primitives — avoids the
  missing close-drain primitive problem entirely.
- Native mutex + two counting semaphores per queue (three kernel
  objects) is the per-instance overhead.
- ISR and native-queue zero-copy optimisations are deferred.
- Wake failure after committed close is fatal — consistent with POSIX
  backend policy.
- The internal mutex is invisible to Queue timeout semantics; timeout
  only governs wait-for-space/wait-for-message.
