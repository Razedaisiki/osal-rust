# System Foundation Slice

## Status

- **Validation:** `Validated` across **Mock**, **POSIX**, and **FreeRTOS**
  (QEMU mps2-an385) — 5 shared contract tests.
- **Policy vocabulary:** Capability states use `Validated` / `Deferred` /
  `N/A` per `docs/documentation-policy.md`.

## Scope

The System foundation slice provides:

- `System::heap_free()` — portable heap introspection
- `System::enter_critical()` — RAII critical-section entry
- `CriticalSectionGuard` — automatic exit on drop
- nested critical-section support
- Mock backend implementation (atomic nesting counter)
- POSIX backend implementation (process-local recursive mutex)
- FreeRTOS backend implementation (`taskENTER_CRITICAL` / `taskEXIT_CRITICAL`)
- shared contract tests (5 tests)
- facade exposure through `osal::prelude::*`

## Non-goals

This slice does **not** provide:

- scheduler start/stop
- ISR yield
- portable interrupt masking API
- heap region enumeration
- board resource reporting
- task lifecycle management
- BSP resource configuration

## Architecture

```
                 osal (facade)
                     |
       +-------------+-------------+-------------+
       |             |             |             |
  PosixSystem   MockSystem   FreeRtosSystem
       |             |             |
 recursive pthread atomic nesting taskENTER_CRITICAL
 mutex (pthread_once) counter      / taskEXIT_CRITICAL
```

## Components

| Layer    | Type               | Location                                  |
|----------|--------------------|-------------------------------------------|
| API      | `System` trait     | `crates/osal-api/src/traits/system.rs`    |
| POSIX    | `PosixSystem`      | `crates/osal-backend-posix/src/system.rs` |
| POSIX    | `PosixRecursiveMutex` | `crates/osal-backend-posix/src/sys/recursive_mutex.rs` |
| Mock     | `MockSystem`       | `crates/osal-backend-mock/src/system.rs`  |
| FreeRTOS | `FreeRtosSystem`   | `crates/osal-backend-freertos/src/system.rs` |
| Facade   | `System` alias     | `crates/osal/src/backend.rs`              |
| Testkit  | System contracts   | `crates/osal-testkit/src/contract/system.rs` |
| Demo     | `osal_demo::system::run()` | `examples/osal-demo/src/system.rs`        |

## Heap model

`heap_free()` returns the number of free heap bytes if the backend
can report it. For the MVP backends, returning `usize::MAX` is valid:

- **Mock** models an unlimited deterministic heap.
- **POSIX** host systems use virtual memory and do not expose a
  portable, stable heap-free value.

FreeRTOS reports kernel heap via `xPortGetFreeHeapSize()`.
Real heap introspection (board memory regions, allocator statistics)
is deferred to the BSP/resource phase.

## Critical-section model

Each call to `enter_critical()` returns a guard. Dropping that guard
exits one nesting level.

Critical sections may be nested. The critical section is fully exited
only after **all** nested guards have been dropped.

| Backend  | Implementation |
|----------|---------------|
| Mock     | `AtomicUsize` nesting counter — validates nesting contract, no thread-level mutual exclusion |
| POSIX    | Process-local recursive `pthread_mutex_t` (`PTHREAD_MUTEX_RECURSIVE`), lazy-initialised via `pthread_once` |
| FreeRTOS | `taskENTER_CRITICAL()` / `taskEXIT_CRITICAL()` (interrupt disable, nesting supported — `FreeRtosCriticalSectionGuard` is `!Send + !Sync` via `PhantomData<Rc<()>>`) |

### Guard hardening

`MockCriticalSectionGuard` and `PosixCriticalSectionGuard` carry a
private `_private: ()` field. This prevents external code from directly
constructing a guard (which would underflow the counter on Mock or call
`pthread_mutex_unlock` on an unheld mutex on POSIX).

`FreeRtosCriticalSectionGuard` uses `PhantomData<Rc<()>>` to make the
guard `!Send + !Sync`, preventing cross-task migration.

Guards are only obtainable through `System::enter_critical()`.

## POSIX recursive mutex

The POSIX backend uses a dedicated `sys::recursive_mutex` module,
separate from the non-recursive `sys::mutex::PosixMutex` used by
`Mutex<T>`. This separation is necessary because:

- `Mutex<T>` is non-recursive by design (ADR 0007).
- `System::enter_critical()` requires nested (recursive) locking.
- Reusing the non-recursive mutex would deadlock on nested
  `enter_critical()` calls.

The recursive mutex is initialised exactly once via `pthread_once`.
All `lock()` / `unlock()` / `init()` calls include `debug_assert_eq!`
return-value checks so that misuse is surfaced in debug/test builds.

## Facade usage

```rust
use osal::prelude::*;

let free = System::heap_free();

{
    let _guard = System::enter_critical();
    let _nested = System::enter_critical();
    // ... critical work ...
} // guards drop in reverse order, each exiting one level
```

## Contract tests

System contract tests verify:

| # | Test | Principle |
|---|------|-----------|
| 1 | `heap_free_is_callable` | `heap_free()` returns without panicking |
| 2 | `enter_critical_returns_guard` | `enter_critical()` returns a valid guard |
| 3 | `critical_section_guard_exits_on_drop` | dropping the guard allows re-entry |
| 4 | `critical_section_supports_nesting` | nested `enter_critical()` is allowed |
| 5 | `nested_guards_drop_in_reverse_order` | guards drop in reverse without deadlock |

## Design decisions

| Decision | Value |
|----------|-------|
| Guard constructibility | Non-constructible (`_private: ()` / `PhantomData`) |
| POSIX critical section | Recursive mutex, not non-recursive `PosixMutex` |
| Mock critical section | Atomic nesting counter, no real lock |
| FreeRTOS critical section | `taskENTER_CRITICAL` / `taskEXIT_CRITICAL`, `!Send + !Sync` guard |
| FreeRTOS heap | `xPortGetFreeHeapSize()` |
| `heap_free()` MVP default | `usize::MAX` (Mock/POSIX) |
| Lazy init | `pthread_once` (POSIX), none needed (Mock/FreeRTOS) |
| ISR / scheduler / yield | Deferred |

## Intentionally deferred

- BSP heap/resource introspection
- Scheduler control (`System::start()` / `System::stop()`)
- ISR-specific system extension traits
- SMP critical sections (`configNUMBER_OF_CORES > 1`)
- Physical MCU validation (deployment validation; not a P7G final-seal gate)

## Next steps

1. SMP support (multi-core critical sections)
2. BSP resource phase (real heap introspection)
3. Physical MCU validation (non-blocking for P7G seal)
