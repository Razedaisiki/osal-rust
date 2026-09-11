# FreeRTOS QEMU MPS2 Cortex-M3 Integration Firmware

OSAL real-kernel validation firmware targeting the ARM MPS2-AN385 board
emulated by QEMU, running FreeRTOS Kernel V11.3.0.

## Purpose

Verify OSAL managed-object contracts on a real FreeRTOS kernel (Cortex-M3,
SysTick, heap_4.c) under QEMU. Each managed-object suite compiles as an
independent firmware profile and is validated via machine-parsable UART
output with a strict verifier.

## Prerequisites

```bash
sudo apt-get install gcc-arm-none-eabi qemu-system-arm make python3
```

## Build

```bash
make                                    # aggregate suite (Mutex + Semaphore + Queue Core)
make CARGO_FEATURES=suite-queue-blocking  # Queue Blocking isolated suite
make CARGO_FEATURES=suite-task            # Task real-kernel contract suite
make CARGO_FEATURES=suite-timer           # Timer real-kernel contract suite
make CARGO_FEATURES=suite-mixed           # Mixed-object real-kernel contract suite
```

Output in `build/`:
- `freertos-qemu-mps2.elf`
- `freertos-qemu-mps2.map`
- `freertos-qemu-mps2.size.txt`

## Run

```bash
make verify                                  # build + symbol check (aggregate)
make verify CARGO_FEATURES=suite-task        # build + symbol check (Task)
make verify CARGO_FEATURES=suite-mixed       # build + symbol check (Mixed)
PROFILE=mixed scripts/run-qemu.sh            # boot in QEMU, verify output
```

## Profiles

| Profile | Cargo Feature | Cases | Description |
|---------|--------------|-------|-------------|
| `suite-aggregate` | (default) | 36 | Mutex (8) + Semaphore (18) + Queue Core (9) + harness (1) |
| `suite-queue-blocking` | `suite-queue-blocking` | 15 | Queue Blocking isolated: 15 required cases = 1 harness + 14 Queue Blocking cases (incl. 4 timeout/wake boundary-race) |
| `suite-task` | `suite-task` | 20 | Task real-kernel contracts (1 harness + 19 Task cases) |
| `suite-timer` | `suite-timer` | 20 | Timer real-kernel contracts (1 harness + 19 Timer cases) |
| `suite-mixed` | `suite-mixed` | 6 | Mixed-object integration: 6 required cases = 1 harness + 5 mixed-object cases |

> Case counts are authoritative in `scripts/verify-boot.py` (`PROFILES`).
> This README copies the current verifier numbers; if they diverge,
> the verifier is the source of truth.

### suite-task

20 required cases with strict profile-aware verifier. Final shutdown +
exact heap recovery required before OBJECT_PASS. Sealing evidence:
TaskExitProbe (unified HWM), DropProbe (exact-once teardown), join-wait
diagnostics (concurrent blocking proof).

### suite-timer

20 required cases with strict profile-aware verifier. Final shutdown +
worker self-delete + exact heap recovery required before OBJECT_PASS.
Sealing evidence: lazy worker identity, one-shot/periodic/coalescing,
callback reentry and outside-lock destruction, clone/in-flight last-drop,
scheduler preconditions, shutdown lease and self-shutdown, same-deadline
(deadline,id) ordering, 56-lifecycle stress with per-round recovery.

### suite-mixed

6 required cases (`verify-boot.py` profile `mixed`) with strict
profile-aware verifier. Final shutdown + worker self-delete + exact
`profile_baseline` heap recovery required before `OBJECT_PASS`.

- `mixed_native_create_rollback` — stage-accurate native handle rollback
  at every Queue/Mutex/Semaphore allocation stage with diagnostic deltas.
- `mixed_resource_pressure_recovery` — real heap pressure via
  `pvPortMalloc`/`vPortFree` (not fault injection), oversized Task Stack
  OOM reaching `xTaskCreate`, and Mutex native-cost subcase.
- `mixed_object_pipeline` — Queue/Task/Timer/Mutex/BinarySemaphore/
  CountingSemaphore composition (Timer → BinarySemaphore → Task A →
  Queue → Task B → Mutex/Counter → CountingSemaphore).
- `mixed_lifecycle_stress` — 16 sequential + 4×2 concurrent pipeline
  lifecycles with per-round/wave heap/task/lease recovery.
- `mixed_shutdown_accounting` — heterogeneous 6-object lease accounting:
  first shutdown `Busy` failure-atomic, per-drop `active_objects` deltas,
  finished-handle lease retention, final exact heap recovery + reinit smoke.

This is the P7G Step 4F sealing suite; verifier gates include
`mixed=true mixed_rollback=true mixed_pressure=true mixed_pipeline=true
mixed_stress=true mixed_shutdown=true` plus task/timer self-delete,
helper cleanup, and `heap_recovered=true`.

### Profile → CI job

| Profile | CI job |
|---------|--------|
| `aggregate` | `freertos-qemu-boot` (aggregate QEMU job) |
| `queue-blocking` | `freertos-qemu-queue-blocking` |
| `task` | `freertos-qemu-task` |
| `timer` | `freertos-qemu-timer` |
| `mixed` | `freertos-qemu-mixed` |

## Boot Protocol

| Marker | Meaning |
|--------|---------|
| `OSAL_BOOT_BEGIN` | Firmware started, about to create boot task |
| `OSAL_BOOT_PASS scheduler=running tick_advanced=true` | Scheduler started, tick advanced |
| `OSAL_BOOT_END status=pass` | Test complete, exiting via semihosting |
| `OSAL_BOOT_FAIL reason=...` | Test failed (see reason) |
| `OSAL_BOOT_FATAL kind=...` | Fatal error (malloc, stack overflow, config assert) |

## Directory Structure

```
integration/freertos-qemu-mps2/
├── README.md
├── Makefile
├── config/
│   └── FreeRTOSConfig.h
├── app/
│   ├── main.c
│   └── hooks.c
├── bsp/
│   ├── console.c / console.h
│   ├── platform.c / platform.h
│   └── qemu_exit.c / qemu_exit.h
├── link/
│   └── mps2_m3.ld
├── scripts/
│   ├── build.sh
│   ├── run-qemu.sh
│   └── verify-boot.py
└── build/
    └── (artifacts, gitignored)
```

## License

Project source files in `app/`, `bsp/`, `config/`, `link/`, and
`scripts/` are part of the OSAL project.

Third-party sources are tracked in `third_party/` with full provenance
(see `third_party/mps2-an385-reference/NOTICE.md`).
