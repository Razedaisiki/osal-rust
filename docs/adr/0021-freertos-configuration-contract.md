# ADR 0021: FreeRTOS Configuration Contract

## Status

Accepted (2026-07-25)
Amended: 2026-07-30 for P7G integration contract alignment.

## Context

FreeRTOS is configured at compile time through `FreeRTOSConfig.h`
macros. The OSAL backend needs to know certain configuration values
(e.g. tick rate, max priorities, whether dynamic allocation is
enabled) to correctly size types, validate parameters, and decide
which code paths are available.

The backend must not guess these values, embed a default config,
or depend on the raw FreeRTOS headers from Rust.

## Decision

### 1. Native build environment variables

Three environment variables point to the application's FreeRTOS
source tree during the native build:

```
OSAL_FREERTOS_KERNEL_INCLUDE  — path to FreeRTOS.h etc.
OSAL_FREERTOS_CONFIG_INCLUDE  — path to FreeRTOSConfig.h
OSAL_FREERTOS_PORT_INCLUDE    — path to portmacro.h etc.
```

All three must be present. The build script (`build.rs`) fails with
a clear error if any is missing. No legacy fallback names are
supported.

### 2. Required configuration macros

The application **MUST** provide a `FreeRTOSConfig.h` with at least:

```c
#define configSUPPORT_DYNAMIC_ALLOCATION 1
#define INCLUDE_xTaskGetSchedulerState    1
#define INCLUDE_vTaskDelay                1
#define configNUMBER_OF_CORES             1
#define configUSE_MUTEXES                 1
#define INCLUDE_vTaskDelete                1
```

The following macros **MUST** be defined and have valid,
non-zero values:

```c
configTICK_RATE_HZ
configMAX_PRIORITIES
configMAX_TASK_NAME_LEN
configMINIMAL_STACK_SIZE
```

### 3. TLS slot requirement

The application **MUST** explicitly define the TLS slot used by the
OSAL Task identity system:

```c
#define OSAL_FREERTOS_TASK_TLS_INDEX  0   // (example)
```

and satisfy:

```c
configNUM_THREAD_LOCAL_STORAGE_POINTERS > OSAL_FREERTOS_TASK_TLS_INDEX
```

The C shim emits `#error` if the macro is missing. A `_Static_assert`
catches out-of-range values at C compile time. There is no default
slot — the application must choose.

### 4. Native FreeRTOS timers are optional

`configUSE_TIMERS` is **not** required. The OSAL Timer subsystem
uses its own Timer Service Task and does not depend on
`timers.c`, `xTimerCreate`, or the native timer daemon.

Legal configurations include `configUSE_TIMERS 0` or the macro
being absent entirely (the capability probe reports
`software_timers = 0` in both cases). Non-zero values are also
valid — the backend simply does not use the native timer subsystem.

### 5. Capability probe

`osal-backend-freertos-sys` exposes a single probe function:

```c
osal_freertos_capability_t osal_freertos_probe_capabilities(void);
```

returning a struct with:

| Field | Source | Type |
|-------|--------|------|
| `tick_rate_hz` | `configTICK_RATE_HZ` | `uint32_t` |
| `max_priorities` | `configMAX_PRIORITIES` | `uint32_t` |
| `max_task_name_len` | `configMAX_TASK_NAME_LEN` | `uint32_t` |
| `tick_bits` | `sizeof(TickType_t) * 8` | `uint8_t` |
| `stack_word_size` | `sizeof(StackType_t)` | `uint8_t` |
| `dynamic_allocation` | `configSUPPORT_DYNAMIC_ALLOCATION` | `uint8_t` (bool) |
| `software_timers` | `configUSE_TIMERS` | `uint8_t` (bool) |
| `minimal_stack_depth_words` | `configMINIMAL_STACK_SIZE` | `uint32_t` |
| `max_stack_depth_words` | max value of stack-depth type | `uint32_t` |
| `tls_pointer_slots` | `configNUM_THREAD_LOCAL_STORAGE_POINTERS` | `uint8_t` |
| `task_tls_index` | `OSAL_FREERTOS_TASK_TLS_INDEX` | `uint8_t` |

The Rust backend calls this once during `initialize()` and caches
the result. Public OSAL APIs never expose raw FreeRTOS macros.

### 6. Missing configuration → compile error

If a required macro is not defined or has an invalid value, the
shim **MUST** emit a `#error` directive at C compile time:

```c
#ifndef configSUPPORT_DYNAMIC_ALLOCATION
#error "FreeRTOSConfig.h must define configSUPPORT_DYNAMIC_ALLOCATION"
#endif
#if configSUPPORT_DYNAMIC_ALLOCATION != 1
#error "OSAL FreeRTOS backend requires configSUPPORT_DYNAMIC_ALLOCATION = 1"
#endif
```

The Rust backend does not perform runtime capability checks for
required features — violations are caught at C compile time.

### 7. Optional capabilities

Capabilities that may vary between valid configurations (e.g.
`configUSE_TIMERS`) are exposed through the capability struct
but do not cause compile errors. The Rust backend may degrade
gracefully (e.g. narrower tick range) or return `Error::Unsupported`
for features that require a specific configuration.

### 8. Tick width

The backend probes `tick_bits` from `sizeof(TickType_t) * 8` at
C compile time. The actual tick-width configuration macro depends
on the locked FreeRTOS Kernel version (`configUSE_16_BIT_TICKS`
for older kernels, `configTICK_TYPE_WIDTH_IN_BITS` for newer ones).
The application must satisfy its Kernel version's own tick-width
requirements; the backend's sole source of truth is
`sizeof(TickType_t)`.

P7G will validate 16-, 32-, and 64-bit `TickType_t` separately.
The probe field and conversion code are designed to accommodate
all widths.

## Consequences

- `osal-backend-freertos-sys` owns the capability probe.
- `osal-backend-freertos` caches `KernelCapabilities` at init time.
- Missing required configuration is a C compile-time error, not a
  Rust runtime error.
- The Rust backend never includes or parses `FreeRTOSConfig.h`.
- Public OSAL APIs contain no FreeRTOS-specific types or macros.
- Configuration changes require recompiling the C shim (and
  therefore the Rust `-sys` crate).
- The TLS slot is explicitly reserved by the application — no
  silent default that could conflict with application TLS usage.
- Native FreeRTOS software timers are not required; the OSAL Timer
  Service Task operates independently.
