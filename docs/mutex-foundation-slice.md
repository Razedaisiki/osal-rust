# Mutex Foundation Slice

## Status

Stabilized (P1.1) — Non-recursive Mutex with corrected memory safety,
handle model, and monotonic clock. Mock, POSIX, and FreeRTOS pass all
core contracts. POSIX additionally passes the blocking and contention tests.
FreeRTOS host fixture passes cross-thread blocking and wakeup tests.

## Architecture

```
                 osal (facade)
                     |
         +-----------+-----------+-----------+
         |                       |           |
  osal-backend-posix    osal-backend-mock   osal-backend-freertos
         |                       |           |
  PosixMutexImpl<T>        MockMutex<T>   FreeRtosMutex<T>
         |                       |           |
    PosixMutex              Rc+UnsafeCell  native mutex
  (PTHREAD_MUTEX_         + Cell<bool>    + spin::Mutex<T>
   ERRORCHECK)                            (priority inheritance)
```

## Components

| Layer | Type | Location |
|-------|------|----------|
| API | `Mutex<T>` trait | `crates/osal-api/src/traits/mutex.rs` |
| POSIX sys | `PosixMutex` (ERRORCHECK) | `crates/osal-backend-posix/src/sys/mutex.rs` |
| POSIX backend | `PosixMutexImpl<T>` | `crates/osal-backend-posix/src/mutex.rs` |
| Mock backend | `MockMutex<T>` | `crates/osal-backend-mock/src/mutex.rs` |
| FreeRTOS backend | `FreeRtosMutex<T>` + Guard | `crates/osal-backend-freertos/src/mutex.rs` |
| FreeRTOS sys | `MutexHandle`, take/give/delete | `crates/osal-backend-freertos-sys/src/lib.rs` |
| FreeRTOS wait | `wait_native()` | `crates/osal-backend-freertos/src/wait.rs` |
| Facade | `Mutex` alias | `crates/osal/src/backend.rs` |
| Testkit | Mutex core contracts | `crates/osal-testkit/src/contract/mutex.rs` |
| Examples | mock_mutex, posix_mutex | `crates/osal/examples/` |

## Design Decisions

| Decision | Value |
|----------|-------|
| Recursive | No — non-recursive, single guard only |
| Guard `!Send` | Yes — PhantomData<*const ()> |
| Guard drop | Only unlock path; no manual unlock |
| Poisoning | Not supported |
| NoWait failure | `Error::LockFailed` |
| After(ZERO) failure | `Error::Timeout` |
| POSIX type | `PTHREAD_MUTEX_ERRORCHECK` |
| POSIX Handle | Arc<PosixMutexInner<T>>, Clone implemented |
| Mock model | `UnsafeCell<T>` + `Cell<bool>` (locked flag) |

## Contract Tests Passing

### MutexCoreContract (Mock + POSIX)

8 tests:
- `create` — creation with initial value
- `lock_unlock` — uncontended lock, guard access, drop releases
- `guard_deref_mut` — mutable access via DerefMut
- `lock_forever` — Forever succeeds uncontended
- `lock_no_wait` — NoWait succeeds uncontended
- `no_second_guard` — second lock while held → LockFailed (non-recursive)
- `clone_shares_state` — clone sees same protected data
- `drop_clone_keeps_alive` — drop one clone, other still works

### MutexBlockingContract (POSIX only)

3 tests:
- `no_wait_fails_when_held` — cross-thread NoWait → LockFailed
- `after_returns_timeout_when_held` — cross-thread After → Timeout
- `forever_woken_by_guard_drop` — cross-thread Forever → woken

## Intentionally Deferred

- Mock blocking/concurrency tests (single execution context; cross-task
  contention not simulated)
- ISR mutex operations (requires extension trait; ADR 0003, ADR 0008)
- Real FreeRTOS kernel runtime tests (QEMU or physical MCU) for
  `Validated` promotion
- `close()` on Mutex (requires ADR; not part of current trait)

## Next Steps

1. FreeRTOS Queue, Task, and Timer primitives (P7D+)
2. Real FreeRTOS kernel validation channel (QEMU/MCU)
3. `RecursiveMutex` trait
