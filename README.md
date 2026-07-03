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
| Mutex           | API only | —        | —        | sys only | skeleton | —        |
| Semaphore       | API only | —        | —        | —        | skeleton | —        |
| System          | API only | —        | —        | —        | skeleton | —        |
| Task            | API only | —        | —        | —        | skeleton | —        |
| Timer           | API only | —        | —        | —        | skeleton | —        |
| Clock           | ✓        | —        | ✓        | —        | ✓        | —        |

**Legend:**
- ✓ — Implemented and tested
- API only — Trait defined, no backend implementation
- sys only — Low-level sys wrapper exists, no trait impl yet
- Partial — Core semantics implemented, blocking deferred
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

## License

Proprietary. See [LICENSE](LICENSE) for details.

## Status

**Queue vertical slice stabilization (P0).**

POSIX Queue and Mock Queue core are implemented and tested. Contract
tests are split into `QueueCoreContract` (all backends) and
`QueueBlockingContract` (POSIX only during P0). ISR operations are
deferred to a future `IsrQueue` extension trait.

CI enforces format, clippy, tests, docs, and feature matrix checks.

See [CHANGELOG.md](CHANGELOG.md) for recent API changes.
