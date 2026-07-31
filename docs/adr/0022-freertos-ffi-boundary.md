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

FreeRTOS native handles are exposed as owned, opaque wrapper types
in the `-sys` crate:

```rust
// In osal-backend-freertos-sys
pub struct MutexHandle(NonNull<c_void>);       // !Copy, Drop deletes
pub struct SemaphoreHandle(NonNull<c_void>);   // !Copy, Drop deletes
pub struct EventGroupHandle(NonNull<c_void>);  // !Copy, Drop deletes
```

These wrappers own the native resource and delete it on drop.
Raw pointer types (`*mut c_void`) are used only for transient
references (e.g. the native task handle returned by
`xTaskGetCurrentTaskHandle`).

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

Task entry points cross the C↔Rust boundary:

- C→Rust task trampolines use `extern "C"` functions in the backend
  crate (not in `-sys`).
- Trampolines **MUST NOT** unwind (panic across FFI is UB).
  The backend uses `panic = "abort"` (workspace default).
- Task context pointers are passed as `*mut c_void` and
  reconstructed via `Box::from_raw` in the trampoline.
- The trampoline owns the context pointer and consumes it after the
  user entry function returns.

Timer callbacks are pure Rust — the Timer Service Task dispatches
them within the backend crate. They do not cross the C FFI boundary.

### 5. Native status code mapping

FreeRTOS returns `BaseType_t` / `pdPASS` / `pdFAIL` or
`pdTRUE` / `pdFALSE` from most APIs. The `-sys` crate translates
these to per-operation status enums:

```rust
pub enum TakeStatus   { Acquired, Timeout, Invalid }
pub enum GiveStatus   { Ok, Full, Invalid }
pub enum TaskCreateStatus { Ok, OutOfMemory, Invalid }
pub enum DelayStatus  { Ok, InvalidTicks, SchedulerStopped }
pub enum EventGroupStatus { Ok, Timeout, Invalid }
```

The C shim returns `uint32_t` status codes; the `-sys` crate
maps each to the appropriate enum. The backend crate maps these
status enums to `osal_api::Error` with semantic equivalence to
the POSIX backend's error mapping.

The Queue and Timer subsystems are self-built Rust models
(ByteQueue + native mutex, Timer Service Task). They map their
own internal state to `osal_api::Error` directly — they do not
go through the C shim for error translation.

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
