# Semaphore Foundation Slice

## Status

Complete — CountingSemaphore and BinarySemaphore are implemented across
the full stack: API traits, portable state machine, Mock backend, POSIX
backend, FreeRTOS backend, contract tests, facade, and examples.

## Architecture

```
                 osal (facade)
                     |
         +-----------+-----------+-----------+
         |                       |           |
  osal-backend-posix    osal-backend-mock   osal-backend-freertos
         |                       |           |
  PosixCountingSemaphore  MockCountingSem.  FreeRtosCountingSem.
         |                       |           |
    Arc<Inner>             Rc<RefCell<>>   Arc<SemaphoreInner>
    mutex+condvar          (single-ctx)    native kernel semaphore
    + portable State                       (kernel count = truth)
```

BinarySemaphore: POSIX and Mock delegate to CountingSemaphore(1, 0);
FreeRTOS uses a dedicated native binary semaphore (`xSemaphoreCreateBinary`).

## Components

| Layer | Type | Location |
|-------|------|----------|
| API | `CountingSemaphore` trait | `crates/osal-api/src/traits/semaphore.rs` |
| API | `BinarySemaphore` trait | `crates/osal-api/src/traits/semaphore.rs` |
| Portable | `CountingSemaphoreState` | `crates/osal-portable/src/counting_semaphore.rs` |
| POSIX | `PosixCountingSemaphore` | `crates/osal-backend-posix/src/semaphore.rs` |
| POSIX | `PosixBinarySemaphore` | `crates/osal-backend-posix/src/semaphore.rs` |
| Mock | `MockCountingSemaphore` | `crates/osal-backend-mock/src/semaphore.rs` |
| Mock | `MockBinarySemaphore` | `crates/osal-backend-mock/src/semaphore.rs` |
| FreeRTOS | `FreeRtosCountingSemaphore` | `crates/osal-backend-freertos/src/semaphore.rs` |
| FreeRTOS | `FreeRtosBinarySemaphore` | `crates/osal-backend-freertos/src/semaphore.rs` |
| FreeRTOS sys | `SemaphoreHandle`, take/give/count | `crates/osal-backend-freertos-sys/src/lib.rs` |
| FreeRTOS wait | `wait_native()` | `crates/osal-backend-freertos/src/wait.rs` |
| Facade | Type aliases | `crates/osal/src/backend.rs` |
| Testkit | Core + blocking contracts | `crates/osal-testkit/src/contract/semaphore.rs` |
| Examples | mock_semaphore, posix_semaphore | `crates/osal/examples/` |

## Design Decisions

| Decision | Value |
|----------|-------|
| ISR | Removed from core traits (ADR 0008) |
| `count()` | `Result<u32>` (snapshot, may fail if lock fails) |
| `max_count()` | `u32` (fixed at construction, no lock) |
| BinarySemaphore query | `is_signaled() -> Result<bool>` |
| BinarySemaphore impl | Delegates to CountingSemaphore(1, 0) |
| Release at max | `Error::Overflow` (count unchanged) |
| Acquire on empty | `Error::Timeout` (NoWait / After(ZERO)) |
| Handle Clone | Rc (Mock), Arc (POSIX) |
| POSIX timed wait | mutex+condvar, `CLOCK_MONOTONIC` deadline |
| Mock Forever | `Error::Unsupported` |
| POSIX wake count | `pthread_cond_signal` (wake ONE) |

## Contract Tests Passing

### CountingSemaphoreCore (Mock + POSIX)

14 tests: creation, bounds validation, acquire/release, overflow,
NoWait timeout, After(ZERO) timeout, After(ZERO) success, failed
acquire preserves count, clone sharing, drop clone preserves resource.

### BinarySemaphoreCore (Mock + POSIX)

9 tests: unsignaled create, release signals, acquire clears, double
release overflow, overflow preserves signal, NoWait timeout,
After(ZERO) timeout, clone sharing, drop clone preserves resource.

### Blocking Contracts (POSIX only)

8 tests (generic over SemaphoreFactory):
- Counting: Forever wakes, After succeeds, After not early, After
  times out, one release one waiter, limit never exceeded
- Binary: Forever wakes, After not early

## Intentionally Deferred

- ISR semaphore operations (requires `IsrSemaphore` extension trait;
  ADR 0003, ADR 0008)
- Mock blocking scheduler emulation (Mock returns `Unsupported`
  for `Forever` on empty)
- Real FreeRTOS kernel runtime tests (QEMU or physical MCU) for
  `Validated` promotion
- Strict FIFO wake ordering
- Named / process-shared semaphores
- Priority inheritance

## Next Steps

1. FreeRTOS Queue, Task, and Timer primitives (P7D+)
2. Real FreeRTOS kernel validation channel (QEMU/MCU)
3. ISR extension traits
