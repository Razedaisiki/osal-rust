# OSAL Architecture

## 1. Overview

OSAL (Operating System Abstraction Layer) is a layered Rust framework for
building portable embedded and real-time applications. It allows you to
write application logic once and run it across different platforms —
POSIX hosts, real-time kernels, and mock environments — by changing a
single Cargo feature flag.

## 2. Design Goals

- **Portable applications**: Application code depends only on the `osal`
  facade crate. Switching platforms is a Cargo feature change, not a
  rewrite.
- **Backend independence**: Backend implementations are isolated behind
  public traits. Adding a new backend requires no changes to application
  code or to other backends.
- **Contract-driven quality**: Every backend must pass the same set of
  behavioral contract tests, ensuring consistent semantics across
  platforms.
- **Clean layering**: Each layer depends only on the layer below it.
  Platform details never leak into the public API.

## 3. Layer Architecture

### 3.1 Current implementation

```
Application
    ↓
osal (facade crate)
    ↓
osal-api (public traits and types)
    ↓
+-------+     +---------------+
| osal-  |     | osal-portable |
| shared |     | (helpers)     |
+-------+     +---------------+
    ↓               ↓
+-----------------------+
| osal-backend-posix    |
| osal-backend-mock     |
+-----------------------+
    ↓
POSIX host / deterministic in-process model
```

### 3.2 Target extension (future)

```
osal-api
    ↓
osal-shared + osal-portable
    ↓
osal-backend-freertos        (future)
    ↓
osal-bsp + board BSP         (future)
    ↓
RTOS / hardware
```

BSP crates (`osal-bsp`, `osal-bsp-linux`) currently exist as deferred
workspace placeholders. They define a future responsibility boundary
but are not part of the active POSIX/Mock MVP and do not yet provide
production board-support functionality.

### 3.3 Crate maturity

| Crate                   | Status               |
| ----------------------- | -------------------- |
| `osal-api`              | Active               |
| `osal-shared`           | Active / stabilizing |
| `osal-portable`         | Active               |
| `osal-backend-posix`    | Active               |
| `osal-backend-mock`     | Active               |
| `osal-testkit`          | Active               |
| `osal` (facade)         | Active               |
| `osal-bsp`              | Skeleton / deferred  |
| `osal-bsp-linux`        | Skeleton / deferred  |
| `osal-backend-freertos` | Planned              |

## 4. Crate Descriptions

### 4.1 `osal-api` — Foundation

The foundation crate. Defines **what** OSAL can do, not **how**.

- Public traits for all OS primitives (Mutex, Semaphore, Queue, Task,
  Timer, Clock, System)
- Shared types: `Error`, `Timeout`, `Result<T>`, `TaskHandle`,
  `Priority`, `RuntimeState`
- Zero runtime dependencies
- `no_std` compatible

Backend crates implement these traits. The `osal` facade re-exports
everything users need.

**Current public traits:** `Queue`, `Mutex<T>`, `CountingSemaphore`,
`BinarySemaphore`, `Clock`, `Timer`, `System`, `Task`, `TaskBuilder`.

**Planned capabilities:** `EventFlags`, ISR extension traits,
FreeRTOS-specific extensions. These are not yet implemented and are not
part of the current backend conformance requirements.

### 4.2 `osal-shared` — OS-Independent Logic

Shared implementation that all backends use:

- Common parameter validation helpers (`validate_queue_capacity`,
  `validate_send_message_size`, etc.)
- Runtime lifecycle state machine (`RuntimeLifecycle`, `RuntimeLease`,
  `InitializeTransition`, `ShutdownTransition`)
- Close-state tracking

A global object ID registry and object table are deferred by
[ADR 0006](adr/0006-object-handle-model.md). The MVP uses strongly
typed handles with backend-appropriate ownership (`Arc`, `Rc`, native
handles) rather than a central numeric-ID registry.

### 4.3 `osal-portable` — Reusable Helpers

Utilities that multiple backends may optionally use:

- Ring buffer implementation (`ByteQueue`)
- Counting semaphore state machine (`CountingSemaphoreState`)
- Timer state machine (`TimerState`)

These are **internal building blocks**, not part of the public API.

### 4.4 `osal-backend-*` — Platform Implementations

Each backend crate implements all `osal-api` traits for a specific
platform:

| Crate | Platform | Use Case |
|-------|----------|----------|
| `osal-backend-posix` | Linux, macOS, POSIX | Development, CI, simulation |
| `osal-backend-mock` | In-process fake | Unit tests, contract verification |
| `osal-backend-freertos` | FreeRTOS | ARM Cortex-M, RISC-V embedded (planned) |

Backends depend on `osal-api`, `osal-shared`, and optionally
`osal-portable`. They must not depend on each other. Each backend
owns its own `RuntimeLifecycle` instance (ADR 0019).

### 4.5 `osal-bsp` + `osal-bsp-*` — Board Support (deferred)

Separates platform hardware configuration from OS backend logic.
**Not part of the current MVP.** Planned responsibilities:

- Boot and startup hooks
- Console / debug output
- Clock and timer hardware access
- Interrupt controller configuration
- Memory and heap region setup
- Resource limits (max tasks, max queues)

### 4.6 `osal-testkit` — Test Infrastructure

Shared testing utilities:

- Contract test harness for running behavior tests against any backend
- Factory traits (`QueueFactory`, `MutexFactory`, `RuntimeFactory`, etc.)
- Assertion helpers for OSAL-specific verification
- Fake clock and fault injection framework

### 4.7 `osal` — Facade

The only crate users depend on:

```toml
[dependencies]
osal = "0.1"
```

Responsibilities:
- Re-export `osal-api` types
- Select backend via facade Cargo features (`backend-posix`, `backend-mock`, future `backend-freertos`)
- Guard against multiple-backend selection at compile time
- Provide `prelude` module for convenient imports
- Expose `initialize()`, `shutdown()`, `runtime_state()` at crate root

## 5. Runtime and Allocation Model

- The public API is designed for `no_std`.
- Dynamic allocation is currently a project-level requirement.
  Crates that need heap allocation declare `extern crate alloc`
  unconditionally. `alloc` is **not** a selectable facade feature.
- Backend selection is independent from Rust `std`.
- The POSIX backend uses native platform APIs through FFI (`libc`)
  and does not require application code to link against Rust `std`.
- A future `std` feature may enable host-only integrations (e.g.
  `impl std::error::Error`), but does not select or enable a backend.

This section is normative and must match
[docs/behavior-contract.md §2](behavior-contract.md).

## 6. Dependency Graph

```
osal-api  ←── osal-shared ←── osal-portable ←── osal-backend-posix
    ↑              ↑
    +── osal-bsp (skeleton) ←── osal-bsp-linux (skeleton)
    +── osal-testkit
    +── osal-backend-mock
    +── osal (facade)
```

No circular dependencies. Each crate depends only on crates below it.

## 7. Feature Flags

### 7.1 Facade-level features

```toml
[features]
default = ["backend-posix"]
backend-posix = ["dep:osal-backend-posix"]
backend-mock = ["dep:osal-backend-mock"]
```

Rules:
- Exactly one backend must be selected at compile time
- `backend-posix` is the default for development convenience
- `backend-mock` is used for testing

### 7.2 Environment features

The `std` Cargo feature is reserved for future host-only integrations.
It is not required to build, test, or use any backend.

`alloc` is **not** a Cargo feature — it is a project-level runtime
assumption. See §5 and behavior-contract §2.

## 8. Naming Conventions

| Aspect | Convention | Example |
|--------|-----------|---------|
| Crate names | `osal-{layer}` | `osal-api`, `osal-backend-posix` |
| Trait names | Noun directly | `pub trait Mutex`, `pub trait Task` |
| Module files | `snake_case.rs` | `clock.rs` |
| Error type | `Error` (no lifetime parameter) | `Error::Timeout` |
| Return type | `Result<(), Error>` for boolean ops | `fn lock(&self) -> Result<()>` |
| ISR methods | `isr_` prefix | `isr_lock()`, `isr_signal()` |
| Backend types | Descriptive names | `Priority` |
| Prelude import | `use osal::prelude::*` | |
| Time types | `core::time::Duration` + `Timeout` enum | `Timeout::After(d)` |

## 9. Error Handling Strategy

OSAL uses a single, flat `Error` enum in `osal-api`:

```rust
pub enum Error {
    OutOfMemory,
    Timeout,
    QueueFull,
    QueueEmpty,
    QueueClosed,
    InvalidMessageSize,
    LockFailed,
    Overflow,
    NotFound,
    InvalidParameter,
    AlreadyInitialized,
    NotInitialized,
    Busy,
    Unsupported,
    Internal(&'static str),
}

pub type Result<T> = core::result::Result<T, Error>;
```

**No lifetime parameter** — keeps the type `Send + Sync + 'static`.

**Boolean-style operations** (lock, signal, wait) return
`Result<(), Error>` instead of a custom boolean type. This is more
idiomatic Rust and integrates with the `?` operator.

**Backend errors** (errno, FreeRTOS status codes) are mapped to OSAL
errors inside backend implementations. Raw platform error codes never
appear in the public API.

## 10. Module Organization Pattern

Within each crate, modules follow this pattern:

```
crates/osal-api/src/
├── lib.rs          # crate root, module declarations
├── error.rs        # Error enum and Result alias
├── time.rs         # Timeout, duration helpers
├── types.rs        # Common type aliases
├── runtime.rs      # RuntimeState enum
├── traits.rs       # trait module declarations
├── traits/
│   ├── mutex.rs
│   ├── semaphore.rs
│   ├── queue.rs
│   ├── task.rs
│   ├── timer.rs
│   ├── clock.rs
│   └── system.rs
└── prelude.rs      # selective re-exports
```

Backend crates mirror the trait structure with concrete implementations:

```
crates/osal-backend-posix/src/
├── lib.rs
├── runtime.rs
├── task.rs
├── mutex.rs
├── semaphore.rs
├── queue.rs
├── timer.rs
├── clock.rs
├── system.rs
├── timer_control.rs
├── timer_service.rs
└── sys/            # thin FFI wrappers
    ├── condvar.rs
    ├── errno.rs
    ├── mutex.rs
    ├── recursive_mutex.rs
    ├── task.rs
    ├── time.rs
    └── tls.rs
```

## 11. Future Backends

To add a new backend:

1. Create `crates/osal-backend-{name}/` with `Cargo.toml` depending on
   `osal-api` + `osal-shared`
2. Implement all `osal-api` traits
3. Own a backend-local `RuntimeLifecycle` instance (ADR 0019)
4. Add the feature flag to `crates/osal/Cargo.toml`
5. Pass the contract test suite from `osal-testkit`

No changes to `osal-api`, `osal-shared`, or existing backends are
required.
