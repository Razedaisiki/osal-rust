# Changelog

## P4 — System Foundation Slice (2026-07-07)

### Added

- `MockSystem` with atomic nesting counter for critical sections.
- `PosixSystem` with process-local recursive `pthread_mutex_t`.
- `sys::recursive_mutex` wrapper (`PTHREAD_MUTEX_RECURSIVE`).
- `SystemFactory` in testkit; 5 system contract tests (heap_free,
  enter_critical, guard-drop re-entry, nesting, reverse drop order).
- `System` facade alias and `System` trait in `osal::prelude`.
- Backend-agnostic `system` facade example.
- `critical_depth_for_test()` helper exposed on Mock for stabilisation tests.

### Notes

- `heap_free()` returns `usize::MAX` for both Mock and POSIX backends.
  Real heap introspection is deferred to the BSP/resource phase.
- POSIX critical sections use a process-local recursive mutex
  (separate from the non-recursive `PosixMutex` used by `Mutex<T>`).
- Mock critical sections model nested entry/exit as an atomic counter
  for deterministic single-context tests.
- Critical sections support nesting; the outermost guard drop fully
  exits.

## P2 — Semaphore Foundation Slice (2026-07-06)

### Added

- `CountingSemaphore` trait with `acquire`/`release`/`max_count`/`count`.
- `BinarySemaphore` trait with `acquire`/`release`/`is_signaled`.
- ADR 0008: ISR Extension Model (ISR removed from core traits).
- `CountingSemaphoreState` portable state machine in `osal-portable`.
- `MockCountingSemaphore` (Rc) and `MockBinarySemaphore` (delegation).
- `PosixCountingSemaphore` (Arc, mutex+condvar, monotonic clock).
- `PosixBinarySemaphore` (delegates to PosixCountingSemaphore).
- 14 CountingCore + 9 BinaryCore contract tests.
- 8 POSIX blocking contract tests (generic over SemaphoreFactory).
- `mock_semaphore` and `posix_semaphore` examples.
- `docs/semaphore-foundation-slice.md`.

### Changed

- **ISR removed from core semaphore traits** (matching Queue P0).
- **`count()` returns `Result<u32>`** (matching Queue `len()`).
- **`is_acquired()` → `is_signaled() -> Result<bool>`**.
- `max_count()` is immutable cached value (no lock).
- Behavior contract §10 fully updated.

## P1.1 — Mutex Correctness Stabilization (2026-07-06)

### Changed (Breaking)

- **`Mutex<T>` is now non-recursive.** Re-locking while a guard is
  alive returns `Error::LockFailed`. The previous recursive + `DerefMut`
  combination was unsound (aliased `&mut T`). Recursive locking is
  deferred to a future `RecursiveMutex` type.

### Fixed

- **Memory safety**: Mock `MockMutexInner` no longer has `unsafe impl
  Send/Sync`. Only one guard can exist at a time.
- **Handle model**: POSIX `PosixMutexImpl<T>` now uses
  `Arc<PosixMutexInner<T>>` and implements `Clone` per ADR 0006.
- **Clock correctness**: POSIX `timed_lock` now uses monotonic clock
  (`clock_gettime(CLOCK_MONOTONIC)` + `try_lock` loop) instead of
  `pthread_mutex_timedlock` which may use `CLOCK_REALTIME`.
- **Sys mutex type**: `PTHREAD_MUTEX_RECURSIVE` → `PTHREAD_MUTEX_ERRORCHECK`.
- **Contract tests**: Removed recursive tests; added non-recursive tests
  (`no_second_guard`, `clone_shares_state`, `drop_clone_keeps_alive`).

### Added

- ADR 0007: Mutex Access Model (non-recursive, single guard).
- `monotonic_now_raw()`, `nanosleep()`, `timespec_ge()` helpers in
  `sys/time.rs`.

## P1 — Mutex Vertical Slice (2026-07-06)

### Added

- ADR 0006: Object Handle Model (strong typed handles, no global ID registry).
- `Mutex<T>` backend implementations:
  - `MockMutex<T>` (`Rc` + `UnsafeCell` + `Cell<usize>`, recursive).
  - `PosixMutexImpl<T>` (`PTHREAD_MUTEX_RECURSIVE`, `try_lock`, `timed_lock`).
- `MutexCoreContract`: 8 tests covering creation, lock/unlock, recursive,
  guard semantics.
- `MutexBlockingContract`: 3 cross-thread tests (POSIX only).
- `mock_mutex` and `posix_mutex` examples.
- `docs/mutex-foundation-slice.md` — architecture, components, deferred items.

### Changed

- `sys/mutex.rs`: switched from `PTHREAD_MUTEX_ERRORCHECK` to
  `PTHREAD_MUTEX_RECURSIVE`.
- `sys/mutex.rs`: added `try_lock()` and `timed_lock()` methods.
- `behavior-contract.md`: fixed POSIX table (Mutex<T> → RECURSIVE);
  added timeout table, error mapping, non-requirements.
- README Mutex row updated from "API only" to fully implemented.
- `object-lifetime.md`: added Guard concept and four-layer object model.

### Fixed

- `docs/queue-foundation-slice.md`: removed "POSIX Queue implementation"
  from Intentionally Deferred; updated contract test counts; updated
  Status to Complete.

## P0 — Queue Vertical Slice Stabilization (2026-07-03)

### Added

- ADRs: error precedence, queue close semantics, ISR API policy, query
  method policy, mock runtime model.
- `Error::Overflow` now covers `capacity * msg_size` overflow.
- Error precedence rules: `InvalidMessageSize` > `QueueClosed` >
  `QueueFull`/`QueueEmpty` > `Timeout` > `Internal`.
- `After(Duration::ZERO)` semantics: resource available → success,
  unavailable → `Error::Timeout`.
- `ByteQueue::is_closed()` public accessor.
- Contract tests split into `QueueCoreContract` and `QueueBlockingContract`.
- Error precedence contract tests.
- CI workflow: format, clippy, test, docs, feature guards.

### Changed

- **`Queue::close()`**: return type `()` → `Result<()>`.
- **`Queue::len()`**: return type `usize` → `Result<usize>`.
- **`Queue::is_empty()`**: return type `bool` → `Result<bool>`.
- **`Queue::is_full()`**: return type `bool` → `Result<bool>`.
- `Queue::capacity()` and `Queue::msg_size()` are now documented as
  non-fallible (fixed at construction).
- `ByteQueue::new()` uses `checked_mul` and `try_reserve_exact` instead
  of the `vec![]` macro; returns `Error::Overflow` on overflow and
  `Error::OutOfMemory` on allocation failure.
- `ByteQueue::try_send()`: error precedence changed — checks
  `InvalidMessageSize` before `QueueClosed`.
- POSIX `QueueInner`: removed duplicate `closed` flag; uses
  `ByteQueue.is_closed()` as sole source.
- POSIX `QueueInner`: cached `capacity` and `message_size` for
  lock-free access.
- POSIX `send()`/`recv()`: no longer double-lock for size validation.
- POSIX `close()`: checks `is_closed()` for idempotency.
- Behavior contract: feature names unified to `backend-posix` /
  `backend-mock`.
- Behavior contract: Semaphore `release()` at max returns
  `Error::Overflow` instead of `Error::InvalidParameter`.
- Contract tests: all error assertions use precise `matches!` with
  exact variant.

### Removed

- **`Queue::isr_send()`** and **`Queue::isr_recv()`** removed from
  core `Queue` trait. ISR operations deferred to future `IsrQueue`
  extension trait.
- ISR methods removed from MockQueue and PosixQueue.
- ISR contract tests (`run_isr_contracts`) removed.
- Behavior contract: ISR descriptions removed from Queue and Mutex
  sections.
- Behavior contract: Mutex `isr_lock()` removed from contract doc
  (never existed in trait).
