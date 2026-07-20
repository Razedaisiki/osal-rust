# OSAL — Operating System Abstraction Layer

A portable, layered abstraction over real-time and general-purpose operating
systems for Rust applications.

## Overview

OSAL provides a unified API for developing multi-platform embedded and
real-time applications. Write your application logic once and run it
across different platforms by switching the backend.

## Capability Matrix

| Capability       | API      | Portable | Mock     | POSIX    | Contract | Facade   |
|-----------------|----------|----------|----------|----------|----------|----------|
| Error           | ✓        | —        | ✓        | ✓        | ✓        | ✓        |
| Timeout         | ✓        | —        | Partial  | ✓        | ✓        | ✓        |
| Queue Core      | ✓        | ✓        | ✓        | ✓        | ✓        | ✓        |
| Queue Blocking  | ✓        | —        | Deferred | ✓        | ✓        | ✓        |
| Queue ISR       | Deferred | —        | —        | —        | —        | —        |
| Mutex           | ✓        | —        | ✓        | ✓        | ✓        | ✓        |
| CountingSemaphore | ✓      | ✓        | ✓        | ✓        | ✓        | ✓        |
| BinarySemaphore | ✓        | ✓        | ✓        | ✓        | ✓        | ✓        |
| Semaphore ISR   | Deferred | —        | —        | —        | —        | —        |
| Clock           | ✓        | ✓        | ✓        | ✓        | ✓        | ✓        |
| Timer           | ✓        | ✓        | ✓        | ✓        | ✓        | ✓        |
| Timer ISR       | Deferred | —        | —        | —        | —        | —        |
| System          | ✓        | —        | ✓        | ✓        | ✓        | ✓        |
| Task            | ✓        | —        | Foundation | Foundation | Smoke    | ✓        |

**Legend:**
- ✓ — Implemented and tested
- API only — Trait defined, no backend implementation
- sys only — Low-level sys wrapper exists, no trait impl yet
- Partial — Core semantics implemented, blocking deferred
- Foundation — Foundation semantic complete (some advanced
  features deferred)
- Smoke — Smoke contract tests exist; advanced concurrency/load
  tests deferred
- Deferred — Planned for future phase
- skeleton — Contract test skeleton exists, not enabled
- — — Not applicable to this layer

## Architecture

OSAL uses a layered crate architecture:

```
Application
    ↓
osal (facade)          ← what you depend on
    ↓
osal-api               ← public traits and types
    ↓
osal-shared + osal-portable  ← shared logic and helpers
    ↓
osal-backend-*         ← platform-specific implementations
    ↓
osal-bsp + osal-bsp-*  ← board support packages
```

See [docs/architecture.md](docs/architecture.md) for details.

## Documentation

### Core design documents

- [Architecture](docs/architecture.md)
- [Behavior Contract](docs/behavior-contract.md) — **source of truth** for backend conformance
- [Object Lifetime](docs/object-lifetime.md)

### Foundation slices

- [Queue Foundation Slice](docs/queue-foundation-slice.md)
- [Mutex Foundation Slice](docs/mutex-foundation-slice.md)
- [Semaphore Foundation Slice](docs/semaphore-foundation-slice.md)
- [Clock and Timer Foundation Slice](docs/clock-timer-foundation-slice.md)
- [System Foundation Slice](docs/system-foundation-slice.md)
- [Task Foundation Slice](docs/task-foundation-slice.md)

### Architecture decisions (ADRs)

- [ADR 0001: Error Precedence](docs/adr/0001-error-precedence.md)
- [ADR 0002: Queue Close-Drain Semantics](docs/adr/0002-queue-close-semantics.md)
- [ADR 0003: ISR API Policy](docs/adr/0003-isr-api-policy.md)
- [ADR 0004: Query Method Policy](docs/adr/0004-query-method-policy.md)
- [ADR 0005: Mock Runtime Model](docs/adr/0005-mock-runtime-model.md)
- [ADR 0006: Object Handle Model](docs/adr/0006-object-handle-model.md)
- [ADR 0007: Mutex Access Model](docs/adr/0007-mutex-access-model.md)
- [ADR 0008: ISR Extension Model](docs/adr/0008-isr-extension-model.md)
- [ADR 0009: Clock Time Domain Model](docs/adr/0009-clock-time-domain-model.md)
- [ADR 0010: Timer Execution Model](docs/adr/0010-timer-execution-model.md)
- [ADR 0011: System Critical Section Model](docs/adr/0011-system-critical-section-model.md)
- [ADR 0012: CI Validation Gates](docs/adr/0012-ci-validation-gates.md)
- [ADR 0013: Task Context and Live Count](docs/adr/0013-task-context-and-live-count.md)
- [ADR 0014: Backend and BSP Responsibility Boundary](docs/adr/0014-backend-bsp-boundary.md)
- [ADR 0015: Runtime Lifecycle](docs/adr/0015-runtime-lifecycle.md)
- [ADR 0016: Linearizable Runtime Lease Accounting](docs/adr/0016-linearizable-runtime-lease.md)

> The English behavior contract (`docs/behavior-contract.md`) is the
> source of truth for backend conformance. Chinese translations are
> supplementary and may lag behind during active MVP development.

## Quick Start

```toml
[dependencies]
osal = "0.1"                                         # POSIX (default)
osal = { version = "0.1", default-features = false,
         features = ["backend-mock"] }               # Mock
```

```rust
use osal::prelude::*;
use core::time::Duration;

fn main() {
    // Create a queue
    let q = Queue::new(8, 4).unwrap();

    // Send a message
    q.send(&1u32.to_le_bytes(), Timeout::NoWait).unwrap();

    // Receive it
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait).unwrap();
}
```

## Examples

Facade examples live under `crates/osal/examples/`. Each example is
capability-oriented and backend-agnostic — the same code runs on any
selected backend.

```bash
# POSIX backend (default)
cargo run -p osal --example queue
cargo run -p osal --example mutex
cargo run -p osal --example semaphore
cargo run -p osal --example timer
cargo run -p osal --example system
cargo run -p osal --example task

# Mock backend
cargo run -p osal --example queue --no-default-features --features backend-mock
cargo run -p osal --example timer --no-default-features --features backend-mock
cargo run -p osal --example task --no-default-features --features backend-mock
```

Backend-specific examples (cross-thread blocking, fault injection,
controlled clock) live under the respective backend crate:

- `crates/osal-backend-mock/examples/`
- `crates/osal-backend-posix/examples/`

## License

Proprietary. See [LICENSE](LICENSE) for details.

## Status

**P0-P5 complete. All MVP primitives are implemented.**

Task foundation supports spawn, join (NoWait / After / Forever),
repeated join with cached exit code, handle, priority, current, and
count. Cancellation, suspend/resume, real priority scheduling, stack
watermark, and deterministic mock scheduling are deferred.

Queue, Mutex, CountingSemaphore, BinarySemaphore, Clock, Timer, and
System are implemented across API, Portable, Mock, POSIX, contract
tests, and facade. Mutex is non-recursive (ADR 0007). System
critical sections use recursive mutex on POSIX and atomic nesting
counter on Mock; heap_free() returns conservative `usize::MAX`.
ISR operations deferred to FreeRTOS phase. Contract tests split
into Core (all backends) and Blocking (POSIX only).

CI enforces format, clippy, tests, docs, and feature matrix checks.

See [CHANGELOG.md](CHANGELOG.md) for recent API changes.
