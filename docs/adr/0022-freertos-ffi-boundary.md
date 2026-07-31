# ADR 0022: FreeRTOS FFI Boundary

## Status

Accepted (2026-07-25)
Amended: 2026-07-30 for P7G integration contract alignment.

## Context

The FreeRTOS backend must call C functions in the FreeRTOS kernel.
Without an explicit FFI boundary, unsafe code, raw pointer casts,
and platform-specific type assumptions tend to spread across the
backend. This ADR defines where `unsafe` is permitted and how the
Rust↔C boundary is structured.

## Decision

### 1. Three-layer FFI architecture

```text
FreeRTOS headers / macros
        ↓
osal_freertos_shim.c           ← C compilation unit, sees FreeRTOS
        ↓                         headers, exposes stable C ABI
osal-backend-freertos-sys       ← unsafe Rust, raw FFI declarations
        ↓
osal-backend-freertos           ← safe Rust, uses -sys types only
```

- The C shim is the **only** compilation unit that includes
  FreeRTOS headers.
- The `-sys` crate is the **only** crate that contains `extern "C"`
  declarations and `unsafe` FFI calls.
- The backend crate uses only safe wrappers and types from `-sys`.

### 2. Opaque handle types

FreeRTOS native handles are exposed as opaque pointers:

```rust
// In osal-backend-freertos-sys
pub type TaskHandle = *mut core::ffi::c_void;
pub type QueueHandle = *mut core::ffi::c_void;
pub type SemaphoreHandle = *mut core::ffi::c_void;
pub type TimerHandle = *mut core::ffi::c_void;
pub type EventGroupHandle = *mut core::ffi::c_void;
```

The Rust backend **MUST NOT**:

- Dereference these pointers
- Cast them to or from integer types (except for debug logging)
- Depend on the layout of the structs they point to (`TCB_t`,
  `Queue_t`, etc.)
- Include or parse FreeRTOS internal headers from Rust

### 3. C shim rules

The C shim (`osal_freertos_shim.c` + `osal_freertos_shim.h`):

- Exposes a stable, versioned C ABI (`osal_freertos_*` prefix)
- Translates between FreeRTOS macros/types and fixed-width C types
- Does **not** store pointers to temporary Rust stack objects
- Does **not** call back into Rust except through registered
  callback function pointers with `void *` context
- All functions are reentrant where the underlying FreeRTOS API is

### 4. Callback safety

Timer callbacks and task entry points cross the C↔Rust boundary:

- C→Rust callbacks use `extern "C"` trampolines in the backend
  crate (not in `-sys`).
- Callbacks **MUST NOT** unwind (panic across FFI is UB).
  The backend uses `panic = "abort"` (workspace default).
- Callback context pointers are passed as `*mut c_void` and
  reconstructed via `Box::from_raw` in the trampoline.
- The trampoline owns the context pointer and is responsible for
  either consuming it (task entry, one-shot callback) or
  preserving it (periodic timer callback).

### 5. Native error code mapping

FreeRTOS returns `BaseType_t` / `pdPASS` / `pdFAIL` or
`pdTRUE` / `pdFALSE` from most APIs. The `-sys` crate translates
these to `Result<(), FreeRtosError>` where:

```rust
enum FreeRtosError {
    Timeout,        // pdFALSE from xQueueReceive with timeout
    QueueFull,      // errQUEUE_FULL
    NotFound,       // invalid handle
    OutOfMemory,    // NULL from pvPortMalloc
    InvalidParameter,
    Internal(u32),  // unexpected / unhandled error code
}
```

The backend crate maps `FreeRtosError` to `osal_api::Error` with
semantic equivalence to the POSIX backend's error mapping.

### 6. Build selection: fixture vs native

The `-sys` crate uses a Cargo feature to select the build mode:

- **`test-fixture` enabled**: compiles against host Rust fixtures
  (`sync_fixture.rs`, `task_fixture.rs`, etc.). No C shim is
  compiled or linked. Used for deterministic host testing and CI.
- **`test-fixture` disabled**: `build.rs` compiles the C shim
  (`osal_freertos_shim.c`). All three `OSAL_FREERTOS_*` include
  environment variables must be set. The final target and link
  environment are determined by the application/integration
  firmware build.

There is no `target_os = "freertos"` compile gate. The backend is
designed to work with bare-metal targets (e.g. `thumbv7m-none-eabi`
where `target_os = "none"`) as well as host-based testing. The
feature flag is the sole build-mode selector.

## Consequences

- `unsafe` is confined to `osal-backend-freertos-sys` and
  trampoline functions in the backend crate.
- The backend crate's public API is safe Rust.
- `extern "C"` declarations exist only in `-sys`.
- C shim is the only compilation unit that `#include`s FreeRTOS.
- Callback unwinding is prevented by `panic = "abort"`.
- Native error codes never appear in `osal-api`.
- CI can build and test the backend crate without a real FreeRTOS
  kernel via the `test-fixture` feature.
- No `target_os` gate blocks bare-metal Rust targets.
