# Queue Foundation Slice

## Status

- **Queue Core (Mock / POSIX / FreeRTOS): Validated** — 18 shared core contract tests
  + 3 clone lifetime tests pass on real QEMU where applicable.
- **Queue Blocking (POSIX / FreeRTOS): Validated** — POSIX host blocking contracts
  + FreeRTOS isolated blocking profile on QEMU mps2-an385 (see Real-kernel Validation below).
- **Queue Blocking (Mock): Deferred** — single execution context; cross-task
  contention not simulated (`Timeout::Forever` on full/empty → `Unsupported`).
- **ISR Queue: Deferred** — `IsrQueue` extension trait.

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
FreeRTOS: host fixture concurrency tests covering cross-thread
wake, wake-one, timeout-race, close broadcast (receiver + sender
single-waiter), scheduler-state preconditions, multi-chunk finite,
and stress cycle.  Shared blocking contract suite integration deferred
until testkit supports generic blocking Queue contracts.  Multi-waiter
close broadcast tests deferred as host-fixture coverage limitation
(the implementation code paths are identical to single-waiter).

### Additional

- `lifetime::run_clone_contracts` — 3 tests (clone shares state, drop clone keeps alive, close affects all clones)
- `fault::run_queue_fault_contracts` — 3 tests (Mock only)

## Real-kernel Validation

Host fixture and QEMU are distinct validation layers; their test
counts are not interchangeable.

- **Aggregate QEMU profile** (`PROFILE=aggregate`, `make`): 36 required
  protocol cases = 1 harness (`harness_native_task`) + 8 Mutex + 18
  Semaphore + 9 Queue Core. Queue Core gates:
  `queue=true queue_fifo=true queue_timeout=true queue_close=true
  queue_suspended=true queue_lease=true`.
- **Queue Blocking isolated profile** (`PROFILE=queue-blocking`,
  `make CARGO_FEATURES=suite-queue-blocking`, CI `freertos-qemu-queue-blocking`):
  **15 required protocol cases = 1 harness + 14 Queue-specific cases**
  (see table below). The isolated profile exists because the aggregate
  suite under accumulated Mutex+Semaphore+Queue pressure cannot reliably
  create additional helper tasks; isolation at 1024 words per helper
  resolves this without a Queue code defect.
- **Timeout/wake boundary-race closure** (P7G Step 4C-3): 4 cases belong
  to the queue-blocking profile (see Boundary-race closure below).

### Queue Blocking profile cases (real FreeRTOS Kernel V11.3.0, QEMU mps2-an385, Cortex-M3)

| Group | Case | What it proves |
|-------|------|----------------|
| Probe | `queue_helper_resource_probe` | recv(After 5 ms)→Timeout, stack HWM ≥ 64, TCB/stack reclamation |
| Wake | `queue_recv_blocking_wake` | Helper blocks, controller sends → helper acquires |
| Wake | `queue_send_blocking_wake` | Helper blocks, controller recvs → helper sends |
| Wake | `queue_recv_forever_wake` | Forever acquires; no spurious Timeout |
| Wake | `queue_send_forever_wake` | Forever send succeeds; controller watchdog |
| Wake-one | `queue_one_send_one_receiver` | Two receivers, one send → exactly one wakes |
| Wake-one | `queue_one_recv_one_sender` | Two senders, one recv → exactly one wakes |
| Close broadcast | `queue_close_broadcast_receivers` | Three receivers, close → all `QueueClosed` |
| Close broadcast | `queue_close_broadcast_senders` | Three senders, close → all `QueueClosed`; M0 drainable; idempotent |
| Throughput | `queue_throughput_cycle` | 64 interleaved NoWait send/recv, FIFO, exact heap recovery |
| Boundary | `queue_recv_timeout_wake_race` | Recv timeout/wake race — injected send at boundary is not lost |
| Boundary | `queue_send_timeout_wake_race` | Send timeout/wake race — injected recv at boundary is not lost |
| Boundary | `queue_recv_close_timeout_priority` | Recv close vs timeout priority — injected close at boundary → `QueueClosed` |
| Boundary | `queue_send_close_timeout_priority` | Send close vs timeout priority — injected close at boundary → `QueueClosed`; M0 drainable |

### Boundary-race closure (Step 4C-3)

`integration/freertos-qemu-mps2/rust/src/cases/queue_blocking.rs` +
`crates/osal-backend-freertos/src/queue_hooks.rs` + feature
`integration-test-hooks`. Helper parks at the timeout boundary via
`TimeoutHookGuard`/`on_timeout_boundary()`; the controller injects the
wake/close before race reconciliation. Verifier `queue-blocking` adds
`queue_timeout_race=true` and `queue_close_timeout_priority=true`.

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
| FreeRTOS | Validated | Validated | Deferred |

## Intentionally Deferred

- ISR queue operations (requires `IsrQueue` extension trait; deferred to FreeRTOS ISR phase)
- Mock blocking scheduler emulation (Mock returns `Error::Unsupported` for `Timeout::Forever` on full/empty)
- FreeRTOS native queue zero-copy optimisation
- Physical MCU validation (deployment validation, not a P7G final-seal gate)

## Next Steps

1. ISR extension traits (`IsrQueue`, `IsrSemaphore`)
2. Deterministic Mock blocking scheduler
3. Optional native Queue / zero-copy optimisation
4. Physical MCU validation (non-blocking for P7G seal)
