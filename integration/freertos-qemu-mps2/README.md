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

make demo DEMO=queue                      # portable demo firmware (see below)
make run-demo DEMO=queue                  # portable demo, build + boot
make run-all-demos                        # all seven portable demos
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

## Firmware Modes

This firmware builds in two modes that are easy to confuse, so they are kept
strictly separate (different Cargo feature, build directory, protocol, and CI
job).

**Validation firmware** (`suite-*`) — conformance:

- executes the OSAL contract suites
- checks kernel behaviour (scheduler, tick, blocking, heap recovery)
- uses the strict boot + object protocol verified by `verify-boot.py`

**Demo firmware** (`demo`) — portable demos:

- runs the user-facing demos from `examples/osal-demo/`
- demonstrates one shared application source across POSIX and FreeRTOS
- uses the smaller `OSAL_DEMO_*` protocol checked by `run-demo.sh`

## Portable OSAL Demos

The demo firmware runs the shared application in `examples/osal-demo/src/` on
a real FreeRTOS kernel, to show the same source running on POSIX and on an
RTOS.

```bash
make run-demo DEMO=mutex
make run-demo DEMO=queue
make run-demo DEMO=semaphore
make run-demo DEMO=system
make run-demo DEMO=task
make run-demo DEMO=timer
make run-demo DEMO=pipeline_demo

make run-all-demos          # all seven in sequence
```

`DEMO` is resolved at compile time by `rust/build.rs` into a single
`osal_demo_entry()` export; there is no per-demo entry point and no `cargo
clean` between demos (`rerun-if-env-changed` handles the selector).

| Shared logic | FreeRTOS glue only |
|--------------|--------------------|
| `examples/osal-demo/src/` | `rust/src/demo_runner.rs` (UART + selector) |

`pipeline_demo` additionally renders a live trace through a platform
reporter; the shared crate defines the event text, so the FreeRTOS UART body
matches the POSIX stdout body. Rendering that trace costs more stack than the
validation path, so demo mode builds the boot task with 2048 words instead of
1600; the validation stack size is deliberately unchanged so its
high-water-mark evidence stays comparable.

| | Validation suites | Portable demos |
|---|---|---|
| Cargo feature | `suite-*` | `demo` |
| C build dir | `build/` | `build/demo/` |
| Protocol | boot + object (verified by `verify-boot.py`) | `OSAL_DEMO_BEGIN/PASS/END` (checked by `scripts/run-demo.sh`) |
| CI job | see table above | `freertos-qemu-demos` |
| Boot task stack | 1600 words | 2048 words |

`demo` and any `suite-*` feature are mutually exclusive at compile time.

### Demo output protocol

Demos use their own machine-readable markers, independent of the boot/object
protocol above. `scripts/run-demo.sh` verifies exactly these.

A successful run looks like this (captured from `DEMO=queue`):

```
OSAL_BOOT_BEGIN
OSAL_BOOT_DIAG stack_hwm_before=2017
OSAL_DEMO_BEGIN name=queue backend=freertos
[queue] sent=1
[queue] received=1
[queue] final_len=0
[queue] roundtrip OK
OSAL_DEMO_PASS name=queue
OSAL_DEMO_END status=pass
OSAL_BOOT_DIAG stack_hwm_after=1139
```

The `stack_hwm_*` values are run-dependent and are not part of the pass
criteria; only the firmware's own 128-word margin check uses them.

| Marker | Meaning |
|--------|---------|
| `OSAL_DEMO_BEGIN name=<demo> backend=freertos` | Demo entry reached |
| `OSAL_DEMO_PASS name=<demo>` | Demo returned a passing report |
| `OSAL_DEMO_END status=pass` | Demo run complete |
| `OSAL_DEMO_FAIL name=<demo> error=...` | Demo failed (the error text follows) |

Pass criteria, all required:

1. QEMU exits 0 (the process exit code is the final authority — a log that
   looks correct but exits non-zero is still a failure, and vice versa);
2. `OSAL_DEMO_BEGIN`, `OSAL_DEMO_PASS`, `OSAL_DEMO_END status=pass` are all
   present, with the right demo name;
3. no `OSAL_DEMO_FAIL`, `OSAL_BOOT_FAIL`, or `OSAL_BOOT_FATAL` anywhere.

Demo mode deliberately does **not** emit `OSAL_BOOT_PASS` or the object
protocol, and does not run `verify-boot.py`.

`pipeline_demo` additionally prints intermediate `[pipeline]` / `[monitor]`
lines between `OSAL_DEMO_BEGIN` and `OSAL_DEMO_PASS`; those are informational
and are not part of the pass criteria.

## Troubleshooting

### QEMU hangs with no UART output (exit 124)

QEMU is invoked with `-nographic -serial stdio`. If stdin is an interactive
terminal, QEMU takes over the tty and the guest never makes progress: no
output at all, then a kill after the 30 s timeout.

Both `run-qemu.sh` and `run-demo.sh` bind stdin to `/dev/null` for exactly
this reason — the firmware needs no console input. CI has no tty, so this
symptom only appears when running from an interactive shell with a
hand-written QEMU command line.

If you write your own invocation, add `< /dev/null`.

### `make run-demo` reports a missing `OSAL_DEMO_BEGIN` marker

Check that the firmware was built in demo mode. A validation firmware boots
and emits `OSAL_BOOT_*`/`OSAL_OBJECT_*` instead, and is not a demo failure.

### Demo fails on `runtime.shutdown` with `Busy`

The FreeRTOS task trampoline publishes completion before it drops the internal
`Arc`s that carry the `RuntimeLease` (ADR 0028 §6). The shared demos therefore
wait, with an upper bound, for the runtime to become quiescent before
shutting down. A persistent `Busy` means a real lease leak.

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
│   ├── run-demo.sh
│   └── verify-boot.py
├── rust/
│   ├── Cargo.toml
│   ├── build.rs           # resolves OSAL_FREERTOS_DEMO in demo mode
│   └── src/
│       ├── lib.rs
│       ├── demo_runner.rs # portable demo UART shell (demo mode only)
│       ├── allocator.rs
│       ├── harness.rs     # suite mode only
│       ├── cases/         # suite mode only
│       └── suite.rs       # suite mode only
└── build/                 # validation artifacts (gitignored)
    └── demo/              # portable demo artifacts (gitignored)
```

## License

Project source files in `app/`, `bsp/`, `config/`, `link/`, and
`scripts/` are part of the OSAL project.

Third-party sources are tracked in `third_party/` with full provenance
(see `third_party/mps2-an385-reference/NOTICE.md`).
