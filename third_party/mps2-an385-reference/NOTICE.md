# Third-Party Notices

## FreeRTOS Kernel

- **Directory**: `freertos-kernel/`
- **Repository**: https://github.com/FreeRTOS/FreeRTOS-Kernel
- **Tag**: V11.3.0
- **Commit**: `9b777ae5c5b8e9e456065a00294d1e5f5f9facf5`
- **License**: MIT (see `freertos-kernel/LICENSE`)

## MPS2-AN385 Platform Reference

- **Directory**: `mps2-an385-reference/`
- **Source repository**: https://github.com/FreeRTOS/FreeRTOS
- **Source commit**: `592732b4d8e8da21f122322de4c421f89e0b4d18`
- **Source path**: `FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/`

Files extracted from the FreeRTOS MPS2 Cortex-M3 QEMU demo:

| File | Original path |
|------|--------------|
| `startup_gcc.c` | `build/gcc/startup_gcc.c` |
| `mps2_m3.ld` | `build/gcc/mps2_m3.ld` |
| `CMSIS/core_cm3.h` | `CMSIS/core_cm3.h` |
| `CMSIS/cmsis.h` | `CMSIS/cmsis.h` |
| `CMSIS/cmsis_compiler.h` | `CMSIS/cmsis_compiler.h` |
| `CMSIS/cmsis_gcc.h` | `CMSIS/cmsis_gcc.h` |
| `CMSIS/cmsis_iccarm.h` | `CMSIS/cmsis_iccarm.h` |
| `CMSIS/cmsis_version.h` | `CMSIS/cmsis_version.h` |
| `CMSIS/CMSDK_CM3.h` | `CMSIS/CMSDK_CM3.h` |
| `CMSIS/mpu_armv7.h` | `CMSIS/mpu_armv7.h` |
| `CMSIS/SMM_MPS2.h` | `CMSIS/SMM_MPS2.h` |

These files are provided under the MIT license. Original copyright
notices are preserved in each file. No modifications have been made
to the vendor source.

The following files from the original demo are intentionally **not**
included: `main.c`, `main_blinky.c`, `main_full.c`, `FreeRTOSConfig.h`,
`IntQueueTimer.c`, `IntQueueTimer.h`, `printf-stdarg.c`, `RegTest.c`,
TraceRecorder, demo tests, `FreeRTOS-Plus`.
