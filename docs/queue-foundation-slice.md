# Queue Foundation Slice

## Status

Complete — Queue vertical slice is stabilized and frozen (p0-complete).
All three backends (Mock, POSIX, FreeRTOS) pass the full contract test
suite. CI enforces format, clippy, tests, docs, and feature matrix checks.

## Architecture

```
                 osal (facade)
                     |
         +-----------+-----------+
         |           |            |
  osal-backend-posix |    osal-backend-freertos
         |           |            |
    PosixQueue        |      FreeRtosQueue
         |           |            |
    ByteQueue +       |      ByteQueue +
    condvar/mutex     |      native mutex +
                      |      sender_wake +
                      |      receiver_wake
                      |
               osal-backend-mock
                      |
                   MockQueue
                      |
                 ByteQueue +
                 Rc<RefCell<>>
```

## Components

| Layer | Type | Location |
|-------|------|----------|
| API | `Queue` trait | `crates/osal-api/src/traits/queue.rs` |
| Portable | `ByteQueue` | `crates/osal-portable/src/byte_queue.rs` |
| Shared | `validate_queue_*` | `crates/osal-shared/src/validation.rs` |
| POSIX | `PosixQueue` | `crates/osal-backend-posix/src/queue.rs` |
| Mock | `MockQueue` | `crates/osal-backend-mock/src/queue.rs` |
| Mock | `MockFaultFactory` | `crates/osal-backend-mock/src/fault.rs` |
| FreeRTOS | `FreeRtosQueue` | `crates/osal-backend-freertos/src/queue.rs` |
| FreeRTOS | `QueueInner` / `QueueState` | `crates/osal-backend-freertos/src/queue.rs` |
| FreeRTOS | `QueueStateGuard` | `crates/osal-backend-freertos/src/queue.rs` |
| FreeRTOS | `WaitBudget` | `crates/osal-backend-freertos/src/wait.rs` |
| FreeRTOS | native mutex + 2 counting semaphores | (per ADR 0027) |
| Facade | `Queue` alias | `crates/osal/src/backend.rs` |
| Testkit | Queue core contracts | `crates/osal-testkit/src/contract/queue/` |
| Testkit | Clone lifetime contracts | `crates/osal-testkit/src/contract/lifetime.rs` |

## Contract Tests Passing

### QueueCoreContract (Mock + POSIX + FreeRTOS)

- `creation::run` — 3 tests (valid create, reject zero capacity, reject zero msg_size)
- `fifo::run` — 4 tests (roundtrip, FIFO order, send full→QueueFull, recv empty→QueueEmpty)
- `error_precedence::run` — 4 tests (wrong send size, wrong recv size, closed+wrong send→InvalidMessageSize, closed+wrong recv→InvalidMessageSize)
- `close::run` — 5 tests (send after close→QueueClosed, recv empty after close→QueueClosed, drain after close, close idempotent, metadata after close)
- `timeout::run` — 2 tests (send timeout on full, recv timeout on empty)

Total: 18 core contract tests across all backends.

### QueueBlockingContract

POSIX: 6 blocking contract tests (full suite).
FreeRTOS: 25 fixture-based concurrency tests covering cross-thread
wake, wake-one, timeout-race, close broadcast (receiver + sender
single-waiter), scheduler-state preconditions, multi-chunk finite,
and stress cycle.  Shared blocking contract suite integration deferred
until testkit supports generic blocking Queue contracts.  Multi-waiter
close broadcast tests deferred as host-fixture coverage limitation
(the implementation code paths are identical to single-waiter).

### Additional

- `lifetime::run_clone_contracts` — 3 tests (clone shares state, drop clone keeps alive, close affects all clones)
- `fault::run_queue_fault_contracts` — 3 tests (Mock only)

## FreeRTOS Architecture (ADR 0027)

`FreeRtosQueue` composes `ByteQueue` (portable ring buffer) with FreeRTOS
native synchronisation primitives:

- **state_mutex** — native FreeRTOS mutex serialising access to `QueueState`
- **sender_wake** — native counting semaphore signalling blocked senders
- **receiver_wake** — native counting semaphore signalling blocked receivers

Native FreeRTOS queue primitives (`xQueueSend`/`xQueueReceive`) are NOT
used — `ByteQueue` is the sole source of truth for message data and close
state.

### Waiter-credit protocol

Each direction maintains two counters:

```
0 <= wake_credits <= waiters
```

- **Normal wake-one**: gives one token if `waiters > credits`.
- **Close broadcast**: gives `waiters - credits` tokens, then sets
  `credits = waiters`.

This prevents stale-token accumulation (each waiter has at most one
unconfirmed token) and wake semaphore overflow.

### WaitBudget

Queue operations may require multiple wait attempts within one API call
(spurious wakeup, condition change, close race). `WaitBudget::Finite`
preserves a single lazily-computed absolute deadline across repeated
`wait_once()` calls.

## Status per backend

| Backend | Queue Core | Queue Blocking | ISR |
|---------|-----------|----------------|-----|
| Mock | Validated | Deferred | N/A |
| POSIX | Validated | Validated | N/A |
| FreeRTOS | Implemented | Implemented | Deferred |

FreeRTOS promotion to Validated requires real FreeRTOS kernel runtime
tests (QEMU or physical MCU).

## Intentionally Deferred

- ISR queue operations (requires `IsrQueue` extension trait; deferred to FreeRTOS ISR phase)
- Mock blocking scheduler emulation (Mock returns `Error::Unsupported` for `Timeout::Forever` on full/empty)
- FreeRTOS native queue zero-copy optimisation
- Real FreeRTOS kernel runtime tests for Validated promotion

## Next Steps

1. FreeRTOS Task and Timer primitives (P7E+)
2. ISR extension traits (IsrQueue, IsrSemaphore)
3. Real FreeRTOS kernel validation (QEMU/MCU)
