# FreeRTOS QEMU MPS2 Cortex-M3 Boot Firmware

C-only FreeRTOS kernel boot test targeting the ARM MPS2-AN385 board
emulated by QEMU.

## Purpose

Verify that FreeRTOS Kernel V11.3.0 boots on a Cortex-M3, schedules
a native C task, advances the SysTick timer, and wakes a task from
`vTaskDelay`. All output goes through UART0 and is machine-parsable.

## Prerequisites

```bash
sudo apt-get install gcc-arm-none-eabi qemu-system-arm make python3
```

## Build

```bash
make
```

Output in `build/`:
- `freertos-qemu-mps2.elf`
- `freertos-qemu-mps2.map`
- `freertos-qemu-mps2.size.txt`

## Run

```bash
make verify                    # build + symbol check
scripts/run-qemu.sh           # boot in QEMU, verify output
```

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
