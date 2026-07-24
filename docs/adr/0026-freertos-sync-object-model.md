# ADR 0026: FreeRTOS Synchronization Object Model

## Status

Accepted (2026-07-26)

## Context

The OSAL handle model (ADR 0006) requires that `Clone` on a handle
creates another reference to the same backend resource, and `Drop`
releases only that reference — the resource is freed when the last
handle drops. For POSIX this is implemented with `Arc<Inner>`.

FreeRTOS native synchronization objects (mutex, counting semaphore,
binary semaphore) are created via `xSemaphoreCreate*()` and destroyed
via `vSemaphoreDelete()`. Each object is a kernel-allocated resource
identified by an opaque `SemaphoreHandle_t`. The backend must bridge
the OSAL `Arc`-based handle model to these FreeRTOS kernel objects.

## Decision

### 1. Native handle ownership

Each Rust object holds exactly one native FreeRTOS handle stored in
the type's `Inner` struct:

```rust
struct MutexInner<T> {
    native: Option<sys::MutexHandle>,
    value: spin::Mutex<T>,
    _lease: RuntimeLease<'static>,
}

struct SemaphoreInner {
    native: Option<sys::SemaphoreHandle>,
    max_count: u32,
    _lease: RuntimeLease<'static>,
}
```

`native` is `Option` so that `Drop` can `take()` the handle and delete
it exactly once. The `Arc` reference count determines when the last
handle drops — only then is the native object deleted.

Rules:

- The native handle is **never** cloned (no `xSemaphoreCreateMutex`
  for every Rust `Clone`).
- `Clone` on the public type only increments the `Arc` strong count.
- Native delete is performed exactly once, in `Inner::drop`, when the
  last `Arc` reference is released.
- The handle is stored as `Option`; `take()` in `Drop` ensures the
  handle is consumed, preventing double-delete.

### 2. Lock order (Mutex)

```
1. Acquire native FreeRTOS mutex  (xSemaphoreTake)
2. Acquire internal spin::Mutex<T> (try_lock — must never block)
3. Return RAII guard
```

The internal `spin::Mutex<T>` exists solely to provide safe `&mut T`
access via the Rust borrow checker. Because step 1 guarantees mutual
exclusion, step 2's `try_lock()` should always succeed. If it does
not, a backend invariant has been violated and the backend panics:

```rust
let value_guard = self.inner.value.try_lock()
    .expect("FreeRTOS mutex invariant: spin lock held after native acquire");
```

This is safer than `UnsafeCell<T>` because a logic error results in a
clear panic rather than undefined behaviour.

### 3. Guard Drop order (Mutex)

```rust
impl<T> Drop for FreeRtosMutexGuard<'_, T> {
    fn drop(&mut self) {
        // 1. Release the Rust value borrow first.
        drop(self.value_guard.take());

        // 2. Release the native mutex.
        if sys::mutex_give(self.native) != sys::GiveStatus::Ok {
            panic!("FreeRTOS mutex give failed after guard release");
        }
    }
}
```

Releasing the value guard **before** the native mutex is essential.
If the native mutex were released first, another task could acquire
it and find the `spin::Mutex<T>` still locked, triggering the
invariant panic in step 2 of the lock order.

### 4. `Send + Sync` conditions

```rust
// Mutex<T>: Send + Sync where T: Send
unsafe impl<T: Send> Send for MutexInner<T> {}
unsafe impl<T: Send> Sync for MutexInner<T> {}
```

FreeRTOS is a multi-task RTOS. Tasks may be migrated across cores
(if SMP) or preempted, so handles must be `Send + Sync` when the
guarded data is `Send`. The POSIX backend uses the same condition
(ADR 0006).

The guard is **always** `!Send + !Sync`:

```rust
pub struct FreeRtosMutexGuard<'a, T> {
    native: &'a sys::MutexHandle,
    value_guard: Option<spin::MutexGuard<'a, T>>,
    _not_send: PhantomData<Rc<()>>,
}
```

`PhantomData<Rc<()>>` provides both `!Send` and `!Sync`.  The guard
must not be moved to another task because the native mutex is owned
by the task that acquired it.

### 5. Native delete constraints

FreeRTOS forbids deleting a mutex that is held by a task. The backend
MUST NOT attempt to detect this at runtime — the application is
responsible for dropping all guards before dropping the last handle.

If a guard outlives the last handle drop:
- The `Inner::drop` runs, calls `sys::mutex_delete()` on a held mutex.
- The behaviour is undefined at the FreeRTOS level.
- The Rust type system cannot prevent this: `Arc::drop` runs when the
  strong count reaches zero, regardless of outstanding borrows.

The backend module documentation MUST state this constraint.

### 6. Semaphore count

The kernel count is the sole source of truth. The backend does NOT
maintain a separate Rust-side count:

```rust
fn count(&self) -> Result<u32> {
    let raw = sys::semaphore_count(&self.inner.native.as_ref()
        .expect("semaphore already deleted"))?;
    Ok(raw as u32)
}
```

`uxSemaphoreGetCount()` returns the current count. For counting
semaphores this is the available permits; for binary semaphores it
is 0 or 1. The value is a snapshot and may change immediately after
return.

### 7. Dynamic allocation requirement

All three object types require `configSUPPORT_DYNAMIC_ALLOCATION == 1`
(already enforced in P7A). Mutex additionally requires
`configUSE_MUTEXES == 1` (enforced in P7C). No static-allocation
path is provided.

### 8. Recursive mutex exclusion

FreeRTOS provides `xSemaphoreCreateRecursiveMutex()` for recursive
locking. ROUSSATL does NOT use this API. The OSAL Mutex trait is
non-recursive (ADR 0007). A future `RecursiveMutex` trait may map
to the FreeRTOS recursive mutex API.

## Consequences

- Native handles are created once, shared via `Arc`, and deleted once
  on last `Drop`.
- `Clone` is cheap — no kernel allocation.
- Mutex guard Drop order prevents a race where a newly-awakened task
  finds the `spin::Mutex<T>` still locked.
- The `spin::Mutex<T>` invariant (must succeed after native acquire)
  converts logic errors into clear panics rather than silent UB.
- Deleting a held mutex is undefined behaviour; the application is
  responsible for drop ordering.
- Semaphore count queries the kernel directly — no dual-state
  synchronisation problem.
- Guards are `!Send + !Sync`, preventing cross-task migration.
- Only task-context operations are supported; ISR variants are
  deferred.
