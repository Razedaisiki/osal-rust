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
- `clock::run_basic_contracts` — 3 tests ✓
- `clock::run_controlled_contracts` — 2 tests ✓

## Intentionally Deferred

- Queue blocking wait/wakeup (requires cooperative scheduler)
- ISR contract tests (requires ISR simulation)
- Fault contract tests (requires integrating fault state into MockQueue)
- POSIX Queue implementation

## Next Steps

1. Integrate fault state into MockQueue → run fault contracts
2. Implement queue wait model (block-on-empty, wake-on-send)
3. POSIX Queue using pthread_cond_t + ByteQueue
4. Apply same pattern to Mutex, Semaphore, Timer
