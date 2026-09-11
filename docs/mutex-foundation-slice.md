# Mutex Foundation Slice

## Status

- **Validation:**
  - Mock / POSIX: `Validated` — all core + (POSIX) blocking contracts pass.
  - FreeRTOS: `Validated` — core contracts via host fixture **and**
    real-kernel validation on **QEMU mps2-an385 / Cortex-M3 /
    FreeRTOS Kernel V11.3.0** in **P7G Step 4A** (8 cases: basic clone,
    non-recursive, nowait/zero, finite timeout, blocking wake, Forever
    wake, scheduler suspended, runtime lease with heap recovery).
  - Host fixture cross-thread tests (Mutex + Semaphore) remain
    deterministic via `BLOCKED_COUNT`.
- **Policy vocabulary:** Foundation-slice capability states use
  `Validated` / `Implemented` / `Deferred` / `N/A` per
  `docs/documentation-policy.md`.

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
| Demo | `osal_demo::mutex::run()` | `examples/osal-demo/src/mutex.rs` |

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
- `RecursiveMutex` trait
- Physical MCU validation (deployment validation; not a P7G seal gate)

## Next Steps

1. `RecursiveMutex` trait
2. Deterministic Mock blocking scheduler
3. ISR extension traits
4. Optional performance/memory optimisations
