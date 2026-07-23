# OSAL — Operating System Abstraction Layer

A portable, layered abstraction over real-time and general-purpose operating
systems for Rust applications.

## Overview

OSAL provides a unified API for developing multi-platform embedded and
real-time applications. Write your application logic once and run it
across different platforms by switching the backend.

## Project Status

**Latest milestone: P7B — FreeRTOS Tick/Time Model, Clock and System.**

The POSIX backend fully implements the current non-deferred `osal-api`
trait surface. The Mock backend implements the same surface with the
exception of blocking Queue contracts (deferred until a deterministic
scheduler is implemented).

FreeRTOS integration: ADRs 0020–0024 define scheduler ownership,
configuration contract, FFI boundary, tick/time model, and System
mapping. The `osal-backend-freertos` crate provides `initialize`/
`shutdown`/`runtime_state` with a C shim capability probe, plus
Clock (`now`/`delay` via coherent tick snapshots) and System
(`heap_free`/`enter_critical` with nesting). FreeRTOS primitive
implementations (Queue, Mutex, Semaphore, Task, Timer) are deferred
to P7C+.

Advanced task controls (cancellation, suspend/resume, real priority
scheduling, stack watermark), ISR extension traits, and production
BSP implementation remain explicitly deferred.

The repository is not yet a production-stable OSAL release.
Public APIs may change before version 1.0.

### Current MVP Scope

**Supported:**

- POSIX backend (`backend-posix`)
- Mock backend (`backend-mock`)
- Queue (core + blocking on POSIX)
- Mutex (non-recursive, ADR 0007)
- CountingSemaphore and BinarySemaphore
- Clock
- Timer
- System operations
- Task foundation (spawn, join with timeout, repeated join, cached exit code)
- Shared backend contract tests (Core, Blocking where applicable)
- Facade backend selection
- Explicit runtime lifecycle (`osal::initialize()` / `osal::shutdown()`)

**Deferred:**

- ISR extension traits (`IsrQueue`, `IsrSemaphore`)
- Deterministic Mock task scheduler
- Task cancellation and suspend/resume
- Real priority scheduling
- Stack watermark
- File system, socket, and shell abstractions
- Production BSP implementation (`osal-bsp` / `osal-bsp-linux` are
  workspace placeholders only)

## Capability Matrix

| Capability        | API       | Mock        | POSIX       | FreeRTOS   | Contract    | Facade    |
|-------------------|-----------|-------------|-------------|------------|-------------|-----------|
| Queue Core        | Validated | Validated   | Validated   | Planned    | Validated   | Validated |
| Queue Blocking    | Validated | Deferred    | Validated   | Planned    | Validated¹  | Validated |
| Queue ISR         | Deferred  | N/A         | N/A         | Deferred   | Deferred    | Deferred  |
| Mutex             | Validated | Validated   | Validated   | Planned    | Validated   | Validated |
| CountingSemaphore | Validated | Validated   | Validated   | Planned    | Validated   | Validated |
| BinarySemaphore   | Validated | Validated   | Validated   | Planned    | Validated   | Validated |
| Semaphore ISR     | Deferred  | N/A         | N/A         | Deferred   | Deferred    | Deferred  |
| Clock             | Validated | Validated   | Validated   | Validated  | Validated   | Validated |
| Timer             | Validated | Validated   | Validated   | Planned    | Validated   | Validated |
| Timer ISR         | Deferred  | N/A         | N/A         | Deferred   | Deferred    | Deferred  |
| System            | Validated | Validated   | Validated   | Validated  | Validated   | Validated |
| Task Foundation   | Validated | Foundation  | Validated   | Planned    | Foundation  | Validated |
| Runtime Lifecycle | Validated | Implemented | Validated   | Implemented| Implemented | Implemented |
| ISR Extensions    | Planned   | N/A         | N/A         | Planned    | Planned     | Planned   |
| BSP               | Planned   | N/A         | N/A         | N/A        | N/A         | N/A       |

**Legend:**

| Status       | Meaning |
|-------------|---------|
| `Validated` | API, implementation, and contract tests complete |
| `Implemented`| Implemented, contract or edge-case verification ongoing |
| `Foundation` | Foundation semantics complete; advanced features deferred |
| `Planned`   | Design exists, implementation not started |
| `Deferred`  | Explicitly deferred to a future phase |
| `N/A`       | Not applicable to this layer |

¹ Blocking Queue contracts currently apply to the POSIX backend only.
Mock blocking is deferred until a deterministic task scheduler is
implemented.

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
osal-bsp + osal-bsp-*  ← board support packages (deferred)
```

See [docs/architecture.md](docs/architecture.md) for the full current
and target architecture, including crate maturity labels.

## Documentation

### Documentation authority

- Rust public APIs and Cargo manifests define the currently available
  compilation surface (signatures, types, features).
- The **[Behavior Contract](docs/behavior-contract.md)** defines
  intended observable backend semantics.
- An implementation that disagrees with the Behavior Contract has a
  **conformance defect**; the implementation does not silently
  redefine the contract.

For semantic conflicts between documents, use this order:

1. **[Behavior Contract](docs/behavior-contract.md)**
2. **[ADRs](docs/adr/)**
3. **[Architecture](docs/architecture.md)**
4. **Foundation slices**
5. **README** (this file)
6. **[CHANGELOG](CHANGELOG.md)**

See [docs/documentation-policy.md](docs/documentation-policy.md) for
the full authority model, update triggers, and status terminology.

### Core design documents

- [Architecture](docs/architecture.md)
- [Behavior Contract](docs/behavior-contract.md) — **source of truth** for backend conformance
- [Documentation Policy](docs/documentation-policy.md) — authority rules and update triggers
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
- [ADR 0017: POSIX no_std Boundary](docs/adr/0017-posix-no-std-boundary.md)
- [ADR 0018: POSIX Timer Service Lifecycle](docs/adr/0018-posix-timer-service-lifecycle.md)
- [ADR 0019: Backend Runtime Ownership Without BSP](docs/adr/0019-backend-runtime-ownership.md)
- [ADR 0020: FreeRTOS Integration Boundary](docs/adr/0020-freertos-integration-boundary.md)
- [ADR 0021: FreeRTOS Configuration Contract](docs/adr/0021-freertos-configuration-contract.md)
- [ADR 0022: FreeRTOS FFI Boundary](docs/adr/0022-freertos-ffi-boundary.md)
- [ADR 0023: FreeRTOS Tick and Time Model](docs/adr/0023-freertos-tick-time-model.md)
- [ADR 0024: FreeRTOS System Mapping](docs/adr/0024-freertos-system-mapping.md)

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

fn main() -> Result<()> {
    // Initialise the runtime before creating any objects.
    osal::initialize()?;

    // Create a queue
    let q = Queue::new(8, 4)?;

    // Send a message
    q.send(&1u32.to_le_bytes(), Timeout::NoWait)?;

    // Receive it
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait)?;

    let value = u32::from_le_bytes(buf);
    assert_eq!(value, 1);

    // Drop all objects before shutting down.
    drop(q);
    osal::shutdown()?;
    Ok(())
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
