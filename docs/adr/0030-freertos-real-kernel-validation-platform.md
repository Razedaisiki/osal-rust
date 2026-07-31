# ADR 0030: FreeRTOS Real-Kernel Validation Platform

## Status

Accepted (2026-07-31)

## Context

The OSAL FreeRTOS backend has been host-contract-verified (P7A–P7F)
but has never run against a real FreeRTOS kernel with hardware
interrupts, a tick timer, and a scheduler. Promotion from
**Implemented** to **Validated** requires integration with a real
FreeRTOS kernel on a real or emulated target.

This ADR selects the validation platform and freezes the kernel
version, target, and integration strategy.

## Decision

### 1. Platform

```
QEMU machine:      mps2-an385
CPU:               cortex-m3 (ARMv7-M)
FreeRTOS port:     portable/GCC/ARM_CM3
Heap:              heap_4.c
```

The MPS2-AN385 is the Cortex-M3 variant of the ARM MPS2 board
family. QEMU emulates it with a working SysTick timer, UART, and
semihosting support — sufficient for scheduler, tick, and task
validation. The official FreeRTOS distribution includes a working
MPS2 demo that uses the same machine, CPU, and port.

### 2. Kernel

```
Repository:  https://github.com/FreeRTOS/FreeRTOS-Kernel
Tag:         V11.3.0
Commit:      9b777ae5c5b8e9e456065a00294d1e5f5f9facf5
```

V11.3.0 is the current formal Kernel release (March 2026).
The kernel is tracked as a direct submodule — no floating branches,
no recursive FreeRTOS distribution.

### 3. Scheduler ownership

The application (integration firmware) owns the scheduler:

- The application creates bootstrap tasks.
- The application calls `vTaskStartScheduler()`.
- The OSAL backend does **not** start or stop the scheduler.
- This matches ADR 0020: the backend is a guest of the kernel.

### 4. Startup entry

- Do not use the official Demo `main.c` or `main_blinky.c`.
- The integration target provides its own `main.c`.
- Startup (`Reset_Handler` / `startup_gcc.c`) and CMSIS headers
  are taken from the official FreeRTOS MPS2 demo with full
  provenance, then frozen as read-only vendor files.
- Linker script (`mps2_m3.ld`) is taken from the same demo and
  adapted only as needed for additional output sections.

FreeRTOS recommends starting from a configured Demo platform and
replacing the application files once the board boots — this is
the same path.

### 5. Output and exit

```
UART0           → test log (machine-parsable)
ARM semihosting → QEMU automatic exit (success or failure)
```

The boot protocol uses structured markers:
`OSAL_BOOT_BEGIN`, `OSAL_BOOT_PASS`, `OSAL_BOOT_END`,
`OSAL_BOOT_FAIL`, `OSAL_BOOT_FATAL`.

### 6. Validation boundary

C-only boot success does **not** promote any OSAL capability
status. It only proves:

> Real FreeRTOS kernel boots and schedules a native C task on
> QEMU Cortex-M3.

OSAL backend validation (Rust staticlib, C shim, heap-backed
allocator, runtime smoke) is a separate future step.

### 7. Naming

All integration code, logs, task names, and firmware identifiers
use neutral naming. No repository-specific names appear in new
code.

## Consequences

- `third_party/freertos-kernel/` — FreeRTOS Kernel V11.3.0 submodule.
- `third_party/mps2-an385-reference/` — frozen vendor platform files
  with full provenance (NOTICE, commit SHA, original paths).
- `integration/freertos-qemu-mps2/` — independent C firmware
  with own `main.c`, config, Makefile, and QEMU scripts.
- `configUSE_TIMERS=0` — native timers excluded; OSAL uses its
  own Timer Service Task.
- `configUSE_RECURSIVE_MUTEXES=0`, `configSUPPORT_STATIC_ALLOCATION=0`
  — match current OSAL backend requirements.
- GitHub Actions: new `freertos-qemu-boot` job runs QEMU boot smoke
  alongside existing host CI jobs.
- No Rust, no C shim, no OSAL runtime — pure C kernel validation.
