# Queue Foundation Slice

## Status

Queue immediate behavior is implemented and verified across the full
stack: portable buffer, validation helpers, mock backend, testkit
contracts, facade, and example.

## Architecture

```
                 osal (facade)
                     |
              osal-backend-mock
              /        |        \
      ByteQueue   validation   CloseFlag
     (portable)   (shared)     (shared)
```

## Components

| Layer | Type | Location |
|-------|------|----------|
| Portable | `ByteQueue` | `crates/osal-portable/src/byte_queue.rs` |
| Shared | `validate_queue_*` | `crates/osal-shared/src/validation.rs` |
| Shared | `CloseFlag` | `crates/osal-shared/src/close_state.rs` |
| Mock | `MockClock` | `crates/osal-backend-mock/src/clock.rs` |
| Mock | `MockQueue` | `crates/osal-backend-mock/src/queue.rs` |
| Mock | `MockFaultFactory` | `crates/osal-backend-mock/src/fault.rs` |
| Facade | `Queue` alias | `crates/osal/src/backend.rs` |

## Contract Tests Passing

```bash
cargo test -p osal-backend-mock
# 5 tests: immediate, lifetime, clone lifetime, clock basic, clock controlled
```

- `queue::run_immediate_contracts` — 8 tests ✓
- `queue::run_lifetime_contracts` — 4 tests ✓
- `lifetime::run_clone_contracts` — 3 tests ✓
- `queue::run_wait_contracts` — 2 tests ✓
- `fault::run_queue_fault_contracts` — 3 tests ✓
- `clock::run_basic_contracts` — 3 tests ✓
- `clock::run_controlled_contracts` — 2 tests ✓

## Intentionally Deferred

- Queue blocking wakeup (recv_blocks_until_send, close_wakes_blocked)
- ISR contract tests (requires ISR simulation)
- POSIX Queue implementation

## Next Steps

1. POSIX Queue using pthread_cond_t + ByteQueue
2. Mutex + Semaphore foundation slice
4. Apply same pattern to Mutex, Semaphore, Timer
