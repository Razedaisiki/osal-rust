# Object Lifetime and Ownership

This document defines ownership, lifetime, and resource management
rules for all public OSAL objects. Every backend must follow these
rules unless a specific deviation is explicitly documented as `Unsupported`.


## 1. Purpose

This section defines ownership, lifetime, and resource management rules
for all public OSAL objects.

The API traits define **what** an object can do. This section defines
**what an object is** — ownership, identity, cleanup, and handle
semantics. Every backend must follow these rules unless a specific
deviation is explicitly documented as `Unsupported`.

## 2. Backend-Owned Objects

All public OSAL objects are **backend-owned handles**. The public API
exposes opaque types; their internal representations are
backend-specific and are **not** part of the public contract.

```
Mutex<T>
  ├── POSIX:    pthread_mutex_t
  ├── FreeRTOS: SemaphoreHandle_t
  └── Mock:     MockMutexState

Queue
  ├── POSIX:    pthread_mutex_t + condvar pair + ring buffer
  ├── FreeRTOS: QueueHandle_t
  └── Mock:     MockQueueState
```

The same pattern applies to `Task`, `Timer`, and `Semaphore`.
Backend native handles must **never** appear in the public API.

## 3. Ownership

Each public object owns exactly one backend resource.

Unless otherwise specified:
- Ownership is **unique** (not shared with other public objects).
- Resource destruction is **deterministic** through RAII.
- Backend resources are released when the public object is dropped.

## 4. Shared Ownership

Some object types may support shared ownership through `Clone`.
Cloning does **not** duplicate the underlying OS resource. Instead,
cloning creates another handle referring to the **same** backend
object.

```
Queue
 ├── Handle A ──┐
 └── Handle B ──┤
                ▼
         Same backend queue
```

Whether an object type is cloneable is part of its API contract.
Types that are not explicitly documented as cloneable must not be
cloned.

## 5. Drop Semantics

Dropping the **last** handle to an object releases the backend
resource. Dropping a clone handle only decrements the reference
count (see [14](#14-reference-counting)). Drop operations must
**not** block indefinitely.

| Object | Required Drop Behavior |
|--------|----------------------|
| Mutex | Destroy backend mutex |
| Semaphore | Destroy backend semaphore |
| Queue | Release queue resources |
| Timer | Stop timer and release resources |
| Task | Release handle only (must not implicitly join) |

Blocking cleanup operations (e.g. waiting for a task to finish) shall
be exposed through **explicit APIs** (`join`, `close`, `stop`) rather
than through `Drop`.

## 6. Task Lifetime

A `Task` handle represents ownership of a backend task object.

- `join()` waits for task completion.
- If `join()` returns `Timeout`, the task handle remains valid and
  may be used again.
- A successful `join()` caches the exit code; subsequent calls return
  it immediately.
- Dropping a `Task` handle must **never** block indefinitely.

## 7. Queue Lifetime

A `Queue` exists until explicitly closed or dropped.

Closing a queue shall:
- Reject future `send` operations (always returns `Error::QueueClosed`).
- Wake all blocked senders (they return `Error::QueueClosed`).
- Wake blocked receivers **only if the queue is empty**; receivers
  blocked on a non-empty queue continue to drain.
- Preserve already enqueued messages — they remain readable via `recv`
  until the queue is empty or dropped.

After close, `recv` returns `Error::QueueClosed` only when the queue
is both closed **and** empty.

## 8. Timer Lifetime

Dropping a timer shall prevent any future callbacks.

Backends shall guarantee that no callback begins execution after the
timer has been successfully destroyed. If callback execution is
already in progress at the time of destruction, backend-specific
synchronization rules apply.

## 9. Backend Independence

Application code must never depend on backend-native object types.
The following are **implementation details**:

- `pthread_mutex_t`
- `pthread_t`
- `SemaphoreHandle_t`
- `TaskHandle_t`
- `QueueHandle_t`

Backends are free to choose any internal representation as long as
the externally observable behavior matches this contract.

## 10. Resource Mapping

Backend implementations may map public objects to native resources
differently. For example:

```
POSIX Queue              FreeRTOS Queue
    ↓                         ↓
pthread mutex             xQueueHandle
pthread condvar
ring buffer
```

Different internal implementations are acceptable provided the
externally observable semantics remain identical.

## 11. Conformance Requirement

Conformance tests validate **observable behavior only**. They must
not assume:

- Specific backend data structures.
- Native handle types.
- Memory layout.
- Implementation algorithms.

Only externally observable semantics are part of the contract.

## 12. Object Identity

Each public object represents exactly **one** backend resource.

- Two cloned handles refer to the **same** backend object.
- Two separately created objects always represent **different**
  backend resources.
- Backends shall **not** duplicate native objects during cloning.

```
let q2 = q1.clone();

q1 ──┐
     ├── Same Queue (not two independent queues)
q2 ──┘
```

This rule is mandatory. Without it, Mock, POSIX, and FreeRTOS
backends would diverge on clone semantics, breaking the portability
guarantee.

## 13. Handle Equality

Two handles that refer to the same backend object must compare equal.

```rust
let q2 = q1.clone();
assert_eq!(q1.handle(), q2.handle());
```

Handles from separately created objects must compare unequal:

```rust
let q1 = Queue::new(8, 4)?;
let q2 = Queue::new(8, 4)?;
assert_ne!(q1.handle(), q2.handle());
```

## 14. Reference Counting

When a type supports `Clone`, all handles contribute equally to the
reference count. The backend resource is destroyed only when the
**last** handle is dropped.

```
Queue::new()  →  refcount = 1
  q2 = q1.clone()  →  refcount = 2
  drop(q1)         →  refcount = 1  (resource alive)
  drop(q2)         →  refcount = 0  (resource destroyed)
```

All handles have equal status — there is no "primary" handle. Any
handle can perform any operation allowed by the object's API.

## 15. Last-Handle Drop Behavior

Explicit `close()` or `delete` operations **must** wake all blocked
tasks and return an appropriate error (`Error::QueueClosed`,
`Error::LockFailed`, etc.).

Last-handle `Drop` is responsible for **resource cleanup only**.
It is not required to wake blocked tasks.

In practice, when the public API uses `&self` methods for blocking
operations:

```rust
queue.recv(&mut buf, Timeout::Forever)?;
```

the blocking caller holds a shared reference to the handle. The
reference count cannot reach zero while a task is blocked, so
last-handle drop cannot occur concurrently with a blocking call.
This is guaranteed by the Rust borrow model for in-process backends
(POSIX, Mock).

For FFI-based backends (FreeRTOS) where blocking calls may use raw
pointers internally, implementations must ensure that wake-before-free
ordering is maintained when a resource is explicitly closed or
deleted from another task.

## 16. Explicit Close with Shared Handles

When `close()` is called on one handle, all handles are affected
equally.

```rust
let q1 = Queue::new(8, 4)?;
let q2 = q1.clone();

q1.close();

// Both handles now see the queue as closed:
q1.send(&data, Timeout::NoWait)?; // Error::QueueClosed
q2.send(&data, Timeout::NoWait)?; // Error::QueueClosed
```

After `close()`, introspection methods (`len()`, `capacity()`,
`is_empty()`, `is_full()`, `msg_size()`) continue to work and
reflect the state at the time of closing.

Close is **idempotent**: calling `close()` multiple times (including
from different handles) is safe and has no additional effect beyond
the first call.

## 17. Signal Safety During Destruction

Backends must ensure that closing or dropping a shared resource does
not leave tasks in an inconsistent state.

Specific requirements:

1. **Wake before deallocation**: blocked tasks must be removed from
   the wait queue before the native resource is freed.
2. **No use-after-free**: once a blocked task is woken with an error,
   it must not access the native resource through the woken path.
3. **Callback safety (Timer)**: after `stop()` or `Drop`, the timer
   callback must not be invoked. If a callback is already executing,
   the backend must ensure it completes without accessing freed memory
   (e.g. through reference counting or a generation counter).
4. **Close-drain ordering (Queue)**: messages already in the queue at
   the time of `close()` remain available to `recv()` until the queue
   is empty or dropped. Closing only prevents new sends; it does not
   discard existing messages.
