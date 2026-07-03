# ADR 0002: Queue Close-Drain Semantics

## Status

Accepted (2026-07-03)

## Context

When `Queue::close()` is called, there are two possible models for
handling already-enqueued messages:

1. **Discard-on-close**: all buffered messages are immediately discarded;
   `recv` returns `QueueClosed` right away.
2. **Drain-on-close**: buffered messages remain available; `recv`
   continues to succeed until the queue is empty, then returns
   `QueueClosed`.

## Decision

OSAL adopts the **drain-on-close** model.

After `close()`:
- `send` always returns `Error::QueueClosed`.
- `recv` succeeds while buffered messages remain.
- `recv` returns `Error::QueueClosed` only when the queue is **both**
  closed **and** empty.
- `close()` is idempotent — calling it multiple times is safe.
- `close()` wakes all blocked senders (they get `QueueClosed`) and all
  blocked receivers on an empty queue (they get `QueueClosed`).

## Rationale

- Drain-on-close allows a graceful shutdown pattern: the producer closes
  the queue, the consumer finishes processing remaining work, then exits.
- Discard-on-close would require out-of-band coordination to know when
  all messages have been processed, defeating the purpose of a queue.
- This matches the behavior of POSIX pipes (`close` write end, read until
  EOF) and Go channels (close, range drains remaining).

## Consequences

- `ByteQueue` must track a `closed` flag separate from the message
  buffer.
- `PosixQueue` must broadcast on both `not_empty` and `not_full` condvars
  on close to wake all blocked senders and receivers.
- `MockQueue` must implement the same drain behavior.
- The `closed` state is terminal — a closed queue can never be reopened.
