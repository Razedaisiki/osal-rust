# ADR 0001: Error Precedence Order

## Status

Accepted (2026-07-03)

## Context

When a single OSAL operation satisfies multiple error conditions
simultaneously (e.g., sending a wrong-sized message to a closed queue),
different backends may return different errors unless the precedence is
explicitly specified. Without a fixed order, portable code cannot
reliably distinguish error conditions.

## Decision

All OSAL backends must return errors in the following fixed order of
precedence (highest first):

```
1. Input parameter validation  (InvalidParameter, InvalidMessageSize)
2. Object state validation     (QueueClosed, NotInitialized)
3. Current resource state      (QueueFull, QueueEmpty, LockFailed)
4. Wait and timeout            (Timeout)
5. Backend system errors       (Internal)
```

For Queue operations specifically:

```
InvalidMessageSize
    ↓
QueueClosed
    ↓
QueueFull / QueueEmpty
    ↓
Timeout
    ↓
Internal
```

## Rationale

- Parameter validation is the cheapest check and should always come first
  — if the caller passed the wrong buffer size, that's what they need to
  fix, regardless of the queue's state.
- Object state (closed, not-initialized) is a terminal condition that
  overrides transient resource states.
- Resource state (full, empty) is transient.
- Timeout is only relevant when waiting.
- `Internal` is a last resort for unexpected platform errors.

## Consequences

- `ByteQueue::try_send()` must check `message.len() != self.message_size`
  **before** `self.closed`.
- All backends must implement this order consistently.
- Contract tests must include explicit error-precedence test cases:
  closed queue + wrong send size → `InvalidMessageSize`.
