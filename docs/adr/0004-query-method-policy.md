# ADR 0004: Query Method Policy (Fallible vs. Infallible)

## Status

Accepted (2026-07-03)

## Context

Queue query methods (`len()`, `is_empty()`, `is_full()`) were originally
defined as infallible, returning plain `usize` or `bool`. However, POSIX
backends must acquire an internal mutex to read runtime state. When the
mutex is poisoned or otherwise fails, the current implementation silently
returns `0` or `false`, masking errors.

`capacity()` and `msg_size()` are fixed at construction time and never
change — they do not require synchronization.

## Decision

| Method | Return type | Rationale |
|--------|-------------|-----------|
| `capacity()` | `usize` | Fixed at construction; no lock needed |
| `msg_size()` | `usize` | Fixed at construction; no lock needed |
| `len()` | `Result<usize>` | Requires lock to read runtime state |
| `is_empty()` | `Result<bool>` | Derived from `len()` |
| `is_full()` | `Result<bool>` | Derived from `len()` and `capacity()` |

`close()` also changes from `()` to `Result<()>` because it acquires the
internal lock and performs condvar broadcast operations.

## Rationale

- `capacity` and `msg_size` are truly infallible for any correct
  implementation. Making them `Result` would be misleading.
- `len` requires synchronization in threaded backends. Returning `0` on
  lock failure silently corrupts the caller's understanding of queue
  state.
- `close()` can fail if the internal lock is poisoned; propagating the
  error is more honest than silently doing nothing.
- This matches the Rust standard library pattern where `Mutex::lock()`
  returns `Result` (poisoning) but `Vec::capacity()` does not.

## Consequences

- POSIX `QueueInner` will cache `capacity` and `msg_size` to avoid
  unnecessary lock acquisition.
- All backends must update their `Queue` trait implementations.
- Contract tests must be updated for the new signatures.
- Callers that previously wrote `q.len()` must now write `q.len()?` or
  `q.len().unwrap()`.
