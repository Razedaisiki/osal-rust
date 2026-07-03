# ADR 0005: Mock Backend Runtime Model and Capability Boundaries

## Status

Accepted (2026-07-03)

## Context

The mock backend provides a deterministic, in-memory simulation of OSAL
primitives for contract testing. However, its current implementation
cannot truly block — `Timeout::Forever` returns `Error::Unsupported`
when the queue is full or empty. This creates a gap between what the
contract specifies and what the mock backend can demonstrate.

## Decision

Contract tests are split into two groups:

### QueueCoreContract (all backends must pass)

- Creation with valid/invalid parameters
- FIFO ordering
- NoWait send/recv on full/empty
- Message size validation
- Close-drain semantics
- Error precedence
- Clone lifecycle

### QueueBlockingContract (only backends with real blocking support)

- `recv(Forever)` woken by `send`
- `send(Forever)` woken by `recv`
- `recv(After)` timeout
- `send(After)` timeout
- `close` wakes blocked senders/receivers
- Multi-sender/multi-receiver correctness
- Spurious wakeup tolerance

### Current backend mapping

| Backend | Core | Blocking |
|---------|------|----------|
| Mock | ✓ | Deferred (returns `Unsupported` for Forever) |
| POSIX | ✓ | ✓ |

## Rationale

- Mock's purpose is deterministic contract verification, not production
  use. It's acceptable for Mock to lack true blocking as long as the
  boundary is clearly documented.
- A full mock scheduler (deterministic task ordering, time advancement,
  context switches) is valuable but out of scope for P0.
- Separating the contracts ensures that tests accurately reflect what
  each backend can do, rather than silently passing fake implementations.

## Consequences

- Mock will not run `QueueBlockingContract` tests during P0.
- Mock's `Forever` behavior (`Error::Unsupported`) is explicitly
  documented and tested.
- The mock scheduler implementation is deferred to a future phase.
- When the mock scheduler is added, Mock can be promoted to pass
  `QueueBlockingContract` as well.
- `QueueCoreContract` alone is sufficient to validate ByteQueue
  correctness, error precedence, and close-drain semantics.
