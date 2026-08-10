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
```

Output in `build/`:
- `freertos-qemu-mps2.elf`
- `freertos-qemu-mps2.map`
- `freertos-qemu-mps2.size.txt`

## Run

```bash
make verify                                  # build + symbol check (aggregate)
make verify CARGO_FEATURES=suite-task        # build + symbol check (Task)
PROFILE=task scripts/run-qemu.sh             # boot in QEMU, verify output
```

## Profiles

| Profile | Cargo Feature | Cases | Description |
|---------|--------------|-------|-------------|
| `suite-aggregate` | (default) | 36 | Mutex (8) + Semaphore (18) + Queue Core (9) + harness (1) |
| `suite-queue-blocking` | `suite-queue-blocking` | 11 | Queue Blocking isolated (independent QEMU run) |
| `suite-task` | `suite-task` | 20 | Task real-kernel contracts (1 harness + 19 Task cases) |
| `suite-timer` | `suite-timer` | 20 | Timer real-kernel contracts (1 harness + 19 Timer cases) |

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
