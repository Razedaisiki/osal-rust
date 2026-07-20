# ADR 0016: Linearizable Runtime Lease Accounting

## Status

Accepted (2026-07-20).  Supersedes the two-atomic double-check
lease algorithm in ADR 0015.

## Context

ADR 0015 originally specified a two-atomic double-check pattern
for `acquire()`:

```rust
check Running
increment count
check Running again
```

This has a window where, after the increment but before the
re-check, a concurrent `shutdown()` can observe `count > 0` and
abort, even though the acquirer will roll back.  The window is
benign for correctness (the acquirer retries or fails), but it
prevents shutdown from making progress if acquirers arrive
frequently.  More importantly, it means the two operations
(`acquire` and `shutdown`) do not share a single linearisation
point, which makes formal reasoning difficult.

## Decision

State and active-object count are packed into a single
`AtomicUsize`:

```text
bits [usize::BITS-1 .. 2] : active object count (saturates at max >> 2)
bits [1 .. 0]              : RuntimeState (2 bits)
```

```
pub struct RuntimeLifecycle {
    word: AtomicUsize,
}
```

Every lifecycle transition and lease operation uses a `CAS` loop
on this single word.  `acquire` and `shutdown` therefore share one
linearisation point — whichever CAS succeeds first wins.

### acquire

```
fn acquire(&self) -> Result<RuntimeLease<'_>> {
    loop {
        let current = self.word.load(Acquire);
        if state(current) != Running { return NotInitialized; }
        let count = object_count(current);
        let next_count = count.checked_add(1).ok_or(Overflow)?;
        let next = encode(Running, next_count);
        if self.word.compare_exchange_weak(current, next, AcqRel, Acquire).is_ok() {
            return Ok(RuntimeLease { lifecycle: self });
        }
    }
}
```

### shutdown

Only transitions from exactly `Running + count == 0`:

```
fn begin_shutdown(&self) -> Result<ShutdownTransition<'_>> {
    loop {
        let current = self.word.load(Acquire);
        match state(current) {
            Uninitialized => return NotInitialized,
            Initializing | ShuttingDown => return Busy,
            Running => {}
        }
        if object_count(current) != 0 { return Busy; }
        let next = encode(ShuttingDown, 0);
        if self.word.compare_exchange_weak(current, next, AcqRel, Acquire).is_ok() {
            return Ok(ShutdownTransition { lifecycle: self, committed: false });
        }
    }
}
```

There are exactly two outcomes for overlapping `acquire` vs
`shutdown`:

1. `acquire` CAS succeeds first → `shutdown` sees `count > 0` →
   returns `Busy`.
2. `shutdown` CAS succeeds first → `acquire` sees state ≠
   `Running` → returns `NotInitialized`.

No intermediate window exists.

### Transition guards commit / rollback

Guards use CAS rather than `store()` so that internal state
corruption is not silently overwritten:

```
fn commit(mut self) {
    let expected = encode(Initializing, 0);
    let desired  = encode(Running, 0);
    self.lifecycle.word.compare_exchange(expected, desired, AcqRel, Acquire).ok();
    self.committed = true;
}
```

Rollback is the symmetric operation.

## Consequences

- `RuntimeLifecycle` becomes a single `AtomicUsize` (no separate
  state + count fields).
- `acquire` and `shutdown` share a single linearisation point.
- Transition guards use CAS for commit and rollback.
- `unsafe impl Send/Sync` is unnecessary — `AtomicUsize` is
  auto-Send + auto-Sync.
- ADR 0015 is annotated with a supersession notice.
- P6B-3 implementation must be rewritten to use this model.
