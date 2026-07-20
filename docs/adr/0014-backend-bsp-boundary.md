# ADR 0014: Backend and BSP Responsibility Boundary

## Status

Accepted (2026-07-20)

## Context

The original OSAL architecture document described `osal-bsp` and
`osal-bsp-linux` as board support packages providing boot hooks,
console, clock, interrupt, memory, and resource configuration.
However, these crates currently contain only comment skeletons.
Several capabilities (`Clock`, `System::enter_critical()`, heap
reporting) have been implemented directly in backends without a
clear rule about which layer owns what.

Before adding runtime lifecycle or FreeRTOS support, the boundary
between backend and BSP must be explicitly defined.

## Decision

### Backend owns

- Task scheduling and thread creation
- Queue, Mutex, Semaphore, Timer
- OS timer service (background thread / ISR dispatch)
- Monotonic clock source (`Clock`)
- OS-level critical sections (`System::enter_critical()`)
- Native error code → `osal_api::Error` mapping
- Backend runtime service start / stop

### BSP owns

- Board / platform metadata (name, vendor, architecture)
- Boot and startup hooks
- Console / debug output
- Heap and memory region information source
- Static resource limits (max tasks, max queues, etc.)
- Chip-level or platform-level initialization
- Panic / fault hooks

### Semantic ownership vs. primitive provider

The OSAL **semantics** of `Clock` and `System::enter_critical()`
belong to the backend. However, the underlying **primitive** may
come from the OS, the RTOS, or the BSP:

| Capability | POSIX primitive | FreeRTOS primitive | Possible BSP role |
|-----------|----------------|-------------------|-------------------|
| `Clock` | `clock_gettime(CLOCK_MONOTONIC)` | RTOS tick | Bare-metal counter / frequency |
| `enter_critical()` | `pthread_mutex_t` (recursive) | interrupt disable / BASEPRI | IRQ mask / chip-level hooks |

The backend **may** accept a BSP-provided clock source or critical-
section primitive via its constructor or configuration, but the
public OSAL trait semantics remain backend-defined.

### BSP composition rules

```
Generic backend depends on osal-bsp abstraction
    (e.g. osal-backend-freertos → osal-bsp)

Concrete BSP is selected at integration time
    (facade Cargo feature, target crate, or application binary)

POSIX MVP default:
    backend-posix feature → osal-backend-posix → osal-bsp-linux

Future FreeRTOS:
    backend-freertos + bsp-stm32f4
```

A generic backend must not hard-code a specific board BSP.

### Runtime initialisation order

```
initialize:
  BSP initialize
  → backend services initialize
  → publish RuntimeState::Running

shutdown (reverse):
  publish RuntimeState::ShuttingDown
  → backend services shutdown
  → BSP shutdown
  → publish RuntimeState::Uninitialized
```

### Dependency direction

```
Application → osal (facade)
                  ↓
             osal-api
                  ↓
        osal-shared + osal-portable
                  ↓
        osal-backend-posix / mock / freertos
                  ↓
        osal-bsp + osal-bsp-linux
                  ↓
        Native OS / RTOS / hardware
```

`osal-bsp` must not depend on `osal-api`. BSP traits return
BSP-specific types; the facade or backend maps them to OSAL types.

The existing `osal-bsp` dependency on `osal-api` is removed.

## Consequences

- `osal-bsp/Cargo.toml`: remove `osal-api` dependency.
- Backend crates depend on `osal-bsp` (abstraction), not on any
  concrete board BSP.
- Concrete BSP selection is a facade/integration concern.
- `Clock` and `System::enter_critical()` semantics remain in
  backends; BSP may supply low-level primitives for bare-metal.
- Runtime init/shutdown follows BSP-first ordering.
- BSP crates are populated with concrete traits.
- `osal-bsp-linux` becomes the default POSIX BSP.
- FreeRTOS backend depends on `osal-bsp` + selected board BSP.
