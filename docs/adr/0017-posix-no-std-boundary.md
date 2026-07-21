# ADR 0017: POSIX Backend `no_std` Boundary

## Status

Accepted (2026-07-21)

## Context

The POSIX backend currently declares `#![no_std]` but unconditionally
`extern crate std;` in its crate root and uses `std::thread_local!`
in `task.rs`.  This means production builds still link the Rust
standard library.  For portability to non-Linux POSIX platforms
(QNX, RTEMS, bare-metal POSIX subsets), the backend must depend
only on the POSIX C ABI.

## Decision

### Production build

The POSIX backend production code uses only:

- `core::*`
- `alloc::*` (`Box`, `Arc`, `String`, `Vec`)
- `libc::*` (pthread, clock, semaphore, etc.)
- `osal_api::*`, `osal_shared::*`, `osal_portable::*`

It must **not** use `std::sync`, `std::thread`, `std::thread_local`,
`std::panic`, `std::time`, or `std::collections`.

### Test allowance

The crate root gates `extern crate std` behind test-only conditions:

```rust
#![no_std]

extern crate alloc;

#[cfg(any(test, feature = "testkit"))]
extern crate std;
```

Unit tests, integration tests, and testkit-driven contract tests may
freely use `std` (threads, barriers, `catch_unwind`, etc.).

### `testkit` is explicitly non-production

`testkit` exists in `Cargo.toml` as:

```toml
[features]
default = []
testkit = ["dep:osal-testkit"]
```

The `std` feature (empty, unused) is removed.  The facade's
`backend-posix` feature must not transitively enable `testkit`.

### Platform minimum

A POSIX port requires:
- POSIX threads (pthread)
- monotonic clock (`clock_gettime(CLOCK_MONOTONIC)`)
- `nanosleep` or equivalent
- C-compatible allocator exposed to Rust
- pointer-width atomics

### Distinct from Linux BSP

The POSIX backend depends only on POSIX C ABI.  Linux-specific
capabilities (`/proc`, `std::io`, host filesystem) belong to
`osal-bsp-linux`, not the backend.  The backend does **not** depend
on any concrete BSP crate.

## Consequences

- Remove `extern crate std;` from `crates/osal-backend-posix/src/lib.rs`
  (gated behind `#[cfg(any(test, feature = "testkit"))]`).
- Replace `std::thread_local!` with `pthread_key_create` /
  `pthread_setspecific` / `pthread_getspecific`.
- Delete the unused `std` feature.
- Add CI check: `cargo check -p osal-backend-posix --no-default-features --lib`.
- Future POSIX BSPs (QNX, RTEMS) can reuse the backend unchanged.
