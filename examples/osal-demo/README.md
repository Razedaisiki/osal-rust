# OSAL Portable Demos

User-facing demonstrations of the OSAL API.

## Purpose

These demos answer one question: *can the same application source run on
POSIX and on a real RTOS by changing only the backend?*

They are **not** conformance tests. Correctness and contract validation live
in `crates/osal-testkit` and the P7G QEMU suites
(`integration/freertos-qemu-mps2/`). If a demo exposes a backend bug, the
backend or the bug report changes — never the contract.

## Shared-source guarantee

The seven demos have **exactly one application implementation**, in
`src/`:

```
src/mutex.rs          src/task.rs
src/queue.rs          src/timer.rs
src/semaphore.rs      src/pipeline_demo.rs
src/system.rs
```

POSIX and FreeRTOS both call the same functions:

| Demo | Both platforms call |
|------|---------------------|
| mutex | `osal_demo::mutex::run()` |
| queue | `osal_demo::queue::run()` |
| semaphore | `osal_demo::semaphore::run()` |
| system | `osal_demo::system::run()` |
| task | `osal_demo::task::run()` |
| timer | `osal_demo::timer::run()` |
| pipeline_demo | `osal_demo::pipeline_demo::run()` |

There is no second implementation anywhere. Backend-specific code is limited
to bootstrap, console/output, and process/firmware termination.

```
                    src/<demo>.rs  (one implementation)
                            │
              ┌─────────────┴─────────────┐
              │                           │
      src/bin/<demo>.rs            integration/freertos-qemu-mps2/
      host main() + stdout          rust/src/demo_runner.rs
              │                     boot task + UART + QEMU exit
              │                           │
        backend-posix               backend-freertos
              │                           │
           pthread                   FreeRTOS kernel
              │                           │
           Linux                  QEMU MPS2 Cortex-M3
```

## How demos report

Demos never print. Each returns a typed report (`MutexReport`,
`QueueReport`, …) whose `Display` implementation lives here too — so both
platforms render the identical body. The platform shell only adds the
protocol markers and the `backend=` field:

```
OSAL_DEMO_BEGIN name=queue backend=posix
[queue] sent=1
[queue] received=1
[queue] final_len=0
[queue] roundtrip OK
OSAL_DEMO_PASS name=queue
OSAL_DEMO_END status=pass
```

The report is the pass/fail artifact. `pipeline_demo` additionally supports
an *optional* live trace, described below.

This crate is `#![no_std]` and depends only on `osal`, `core`, and `alloc`.

## POSIX

```bash
cargo run -p osal-demo --bin mutex
cargo run -p osal-demo --bin queue
cargo run -p osal-demo --bin semaphore
cargo run -p osal-demo --bin system
cargo run -p osal-demo --bin task
cargo run -p osal-demo --bin timer
cargo run -p osal-demo --bin pipeline_demo
```

Exit code 0 means pass; a non-zero exit means the demo failed.

## FreeRTOS (QEMU MPS2 / Cortex-M3)

```bash
make -C integration/freertos-qemu-mps2 run-demo DEMO=mutex
make -C integration/freertos-qemu-mps2 run-demo DEMO=queue
make -C integration/freertos-qemu-mps2 run-demo DEMO=semaphore
make -C integration/freertos-qemu-mps2 run-demo DEMO=system
make -C integration/freertos-qemu-mps2 run-demo DEMO=task
make -C integration/freertos-qemu-mps2 run-demo DEMO=timer
make -C integration/freertos-qemu-mps2 run-demo DEMO=pipeline_demo

# or all seven in sequence
make -C integration/freertos-qemu-mps2 run-all-demos
```

Demo firmware artifacts live in `integration/freertos-qemu-mps2/build/demo/`
and do not collide with the validation firmware in `build/`.

## Mock

All demos except `pipeline_demo` run on the Mock backend:

```bash
cargo run -p osal-demo --bin queue --no-default-features --features backend-mock
```

`pipeline_demo` is gated behind the `pipeline` capability feature (enabled
by `backend-posix` and `backend-freertos`, not by `backend-mock`). It spawns
OSAL tasks sharing state through `Arc`, which requires `Send` task, queue,
and semaphore objects; the Mock backend is `Rc`-based with a single execution
context and no deterministic scheduler, so it cannot compile it. This is a
capability gate, not a backend fork — the demo source contains no `cfg`.

## pipeline_demo

```
  Producer 0 ─┐
              ├──> Queue ──> Consumer 0 ─┐
  Producer 1 ─┘               Consumer 1 ─┼──> Mutex<Stats>
                              Consumer 2 ─┘
  Timer ──callback──> Monitor ──reads──> Stats
  Supervisor: START_BIT → phase 1 → change period → phase 2 → STOP_BIT
```

It exercises `Queue`, `Mutex`, `CountingSemaphore`, `Task`, `Timer`, and
`Clock` together, and is the strongest single demonstration of the
shared-source property.

Absolute counters are timing-dependent and legitimately differ between
platforms; only these invariants are asserted:

```
produced > 0
consumed > 0
consumed <= produced
checksum_error == 0
timer_fires > 0
monitor_samples > 0
worker_error == 0
```

Worker tasks record the first failure in an atomic slot instead of panicking
(the firmware runs `panic = "abort"`), and the supervisor joins with a finite
timeout so a demo bug cannot hang CI or QEMU forever.

### Optional live trace

`pipeline_demo::run()` is silent. `pipeline_demo::run_with_reporter(&r)`
additionally emits `PipelineEvent`s to a caller-supplied `PipelineReporter`,
which is how both platforms show a live trace without this crate knowing
anything about output:

```
[pipeline] init queue=128 packet=16
[pipeline] worker producer-0 started
...
[pipeline] started
[monitor] tick=1022 produced=84 consumed=3 dropped=0 timeout=0 checksum_error=0
[monitor] tick=2005 produced=164 consumed=102 dropped=0 timeout=0 checksum_error=0
[pipeline] stopping
[summary] produced=484 consumed=484 dropped=0
```

The event *text* comes from `PipelineEvent`'s `Display` here, so the two
platforms produce the same body; only the line ending differs (the UART
shell writes CRLF). Reporters live in platform code:

| Platform | Reporter | Sink |
|----------|----------|------|
| POSIX | `PosixPipelineReporter` in `src/bin/pipeline_demo.rs` | stdout |
| FreeRTOS | `FreertosReporter` in `integration/.../rust/src/demo_runner.rs` | UART |

Two properties are worth knowing:

- **A sample, not a firehose.** Monitor events are paced by the heartbeat
  timer (1 s, then 500 ms), not by packet flow, so the trace cannot saturate
  a UART. The supervisor only bounds how quickly a sample becomes visible.
- **Borrowed, not owned.** `run_with_reporter` takes `&R` because every event
  is emitted from the supervisor's own task. `TaskBuilder::spawn` needs a
  `'static` closure, so a reporter captured by a worker task could not be a
  plain reference. Emitting from one task also rules out interleaved writes.

Invariants are still checked on the final report, not on trace events, so a
trace never weakens the pass/fail criteria.

## Relationship to the validation suites

| Layer | Purpose |
|-------|---------|
| `crates/osal-testkit` contracts | Backend conformance (shared contracts) |
| `integration/freertos-qemu-mps2` `suite-*` | Real-kernel conformance on QEMU |
| `examples/osal-demo` | User-facing portability demonstration |

Demos do not use `verify-boot.py`; they have their own three-marker protocol
checked by `scripts/run-demo.sh`.
