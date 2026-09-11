# Engineering Handover

Orientation for whoever picks this repository up next. Read this first, then
[`docs/architecture.md`](architecture.md) and
[`docs/behavior-contract.md`](behavior-contract.md).

It answers three questions the other documents do not: *what is where*,
*why is it shaped this way*, and *how do I know it works*.

## 1. What this project is

OSAL (Operating System Abstraction Layer) is a layered Rust abstraction over
RTOS and general-purpose OS primitives. Application code is written once
against the `osal` facade; the backend is chosen by a Cargo feature.

Three backends exist today: POSIX (development/CI), Mock (deterministic,
single-context), and FreeRTOS (real kernel, QEMU-validated).

## 2. Layout

```
crates/
  osal-api             public traits + types (Error, Timeout, TaskHandle, ...)
  osal-shared          runtime lifecycle, lease accounting, validation helpers
  osal-portable        ByteQueue, CountingSemaphoreState, TimerState, tick/time
  osal-backend-posix   pthread-based backend
  osal-backend-mock    Rc/RefCell deterministic backend
  osal-backend-freertos-sys   the ONLY crate allowed to do extern "C"
  osal-backend-freertos       FreeRTOS backend
  osal                 the facade users depend on
  osal-testkit         contract tests + factories
  osal-bsp / -linux    placeholders (no runtime logic)
examples/
  osal-demo            portable demos, ONE implementation shared by POSIX + FreeRTOS
integration/
  freertos-qemu-mps2   QEMU firmware: validation suites AND portable demos
docs/                  behavior contract, ADRs, foundation slices, policy
tests/                 FreeRTOS native header fixtures for CI compile checks
```

Dependency direction is one-way and enforced by Cargo: `osal-demo -> osal ->
osal-api`. Nothing depends on `osal-demo`.

## 3. Two things that are easy to confuse

### 3.1 Validation suites vs portable demos

Both live in `integration/freertos-qemu-mps2/`, but they are different
firmware builds:

| | Validation suites | Portable demos |
|---|---|---|
| Cargo feature | `suite-*` | `demo` |
| Purpose | backend conformance | user-facing demonstration |
| Protocol | boot + object (`verify-boot.py`) | `OSAL_DEMO_*` (`run-demo.sh`) |
| Artifacts | `build/` | `build/demo/` |
| CI jobs | `freertos-qemu-{boot,queue-blocking,task,timer,mixed}` | `freertos-qemu-demos` |

They are mutually exclusive at compile time. **Do not** merge them: a demo
failure means "the demo is wrong", a suite failure means "the backend is
wrong", and they must stay distinguishable.

### 3.2 Document authority

When documents disagree, the order in
[`docs/documentation-policy.md`](documentation-policy.md) applies:
`behavior-contract.md` > ADRs > `architecture.md` > foundation slices >
`README.md` > `CHANGELOG.md`.

The behavior contract is normative. If code disagrees with it, that is a
**conformance defect** — fix the code, or write a new ADR. Never edit the
contract to match the code.

## 4. Design decisions worth preserving

These are the ones most likely to be "cleaned up" by someone who has not been
told why they exist.

1. **Demo logic is platform-independent.** `examples/osal-demo/src/**` may use
   only `osal`, `core`, and `alloc`. No `#[cfg(feature = "backend-*")]`, no
   `println!`, no native APIs. Platform differences live only in the runners
   (`src/bin/*`, `integration/.../rust/src/demo_runner.rs`). There is exactly
   one implementation per demo — if you are about to add
   `mutex_freertos.rs`, stop.

2. **Demos return reports, not output.** A demo returns
   `DemoResult<Report>`; the report's `Display` lives in the shared crate, so
   both platforms render the same body. Errors are typed
   (`DemoError::{Osal, Check, Worker}`) rather than panics: the firmware runs
   `panic = "abort"`, so a demo assertion would kill the board instead of
   reporting.

3. **`pipeline_demo` has an optional trace sink, not `println!`.**
   `run_with_reporter(&R)` emits `PipelineEvent`s; `run()` is
   `run_with_reporter(&NullReporter)`. Two consequences that look odd but are
   deliberate:
   - the reporter is **borrowed**, because events are emitted from the
     supervisor's own task and `TaskBuilder::spawn` requires a `'static`
     closure;
   - the **monitor task does not print** — it bumps `monitor_samples` and the
     supervisor renders. One writer means no interleaved UART output.

4. **FreeRTOS task teardown is asynchronous.** The trampoline publishes
   completion *before* dropping the `Arc`s that hold the `RuntimeLease`
   (ADR 0028 §6), so a joiner can see `Busy` from `shutdown()` for a few
   ticks. The demos wait, with an upper bound, for quiescence. A persistent
   `Busy` is a real leak.

5. **Demo firmware uses a bigger boot stack** (2048 vs 1600 words) because
   rendering the trace costs more stack than the validation path. The
   validation size is intentionally unchanged so its high-water-mark evidence
   stays comparable across runs.

6. **QEMU runs bind stdin to `/dev/null`.** With `-nographic -serial stdio`
   and an interactive stdin, QEMU grabs the tty and the guest produces no
   output at all (exit 124). CI has no tty, so this only bites interactive
   users. See the integration README's Troubleshooting section.

7. **`osal-backend-freertos-sys` is the only FFI crate.** ADR 0022. Backends
   never call `extern "C"` directly.

8. **Build artifacts are keyed by profile, but C objects are not.**
   Switching `CARGO_FEATURES` changes both Rust features and C `CFLAGS`, and
   `make` only tracks the former. Always use `scripts/build.sh` (which runs
   `make clean`) when changing profiles; a bare `make` can silently relink
   objects from the previous profile. Demo mode sidesteps this with its own
   `build/demo` directory.

## 5. How to know it works

```bash
# Host: all unit + contract tests
cargo test --workspace --features testkit -- --test-threads=1

# Host gates (CI runs these)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features testkit -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --features testkit

# Portable demos — POSIX
cargo run -p osal-demo --bin pipeline_demo

# Portable demos — FreeRTOS QEMU (all seven)
make -C integration/freertos-qemu-mps2 run-all-demos

# Validation suite — build a profile, then boot + verify it.
# Use scripts/build.sh rather than a bare `make`: it runs `make clean` first.
# Switching CARGO_FEATURES changes CFLAGS, and make does not track CFLAGS, so
# a bare make can relink objects compiled for the previous profile and fail
# with undefined `osal_test_*` symbols.
CARGO_FEATURES=suite-aggregate integration/freertos-qemu-mps2/scripts/build.sh
PROFILE=aggregate integration/freertos-qemu-mps2/scripts/run-qemu.sh

# Another profile (build.sh cleans, so switching is safe):
CARGO_FEATURES=suite-mixed integration/freertos-qemu-mps2/scripts/build.sh
PROFILE=mixed integration/freertos-qemu-mps2/scripts/run-qemu.sh
```

`CARGO_FEATURES` and `PROFILE` must name the **same** profile, otherwise the
verifier looks for cases the built firmware does not contain.

Requirements: `gcc-arm-none-eabi`, `qemu-system-arm`, the
`thumbv7m-none-eabi` Rust target, and the initialized
`third_party/freertos-kernel` submodule.

### Current verification status

**P7G (FreeRTOS real-kernel integration and validation) is closed.**

- CI run [34569160234](https://github.com/Razedaisiki/osal-rust/actions/runs/34569160234)
  on commit `50d944a`: all 19 jobs green.
- Host gates: `format`, `clippy`, `test`, `rustdoc`, `feature-guards`,
  `task-tests`.
- QEMU profiles: `aggregate`, `queue-blocking`, `task`, `timer`, `mixed`.
- Demo jobs: `host-demos` (7) and `freertos-qemu-demos`.

**Outstanding, by design:**

- Physical MCU validation (QEMU only so far; not a P7G seal condition).
- ISR extension traits, deterministic Mock scheduler, task cancellation /
  suspend-resume / priority scheduling / stack watermark, production BSP.

### Known flakiness

`osal-backend-freertos` test `timer_scheduling::earlier_timer_wakes_worker_from_long_deadline_wait`
is timing-sensitive in Virtual mode and has been observed to fail
occasionally under load while passing on re-run. It has not been fixed. If CI
goes red on that single test, re-run before investigating.

## 6. Where to change things

| Change | Touch |
|---|---|
| New OSAL primitive | `osal-api` trait → backends → `osal-testkit` contract → behavior contract (§) + ADR |
| New backend | new `osal-backend-*` with its own `RuntimeLifecycle` (ADR 0019) |
| New demo | `examples/osal-demo/src/<name>.rs` + thin `src/bin/<name>.rs` + a `build.rs` selector arm + `VALID_DEMOS` in the Makefile |
| New validation profile | `rust/src/cases/` + `suite.rs` + `verify-boot.py` `PROFILES` + Makefile + CI job |

Do not add a second demo implementation, and do not weaken the behavior
contract or the verifier's case lists to make something pass.

## 7. Status vocabulary

Use only the terms in
[`docs/documentation-policy.md`](documentation-policy.md):

- capability: `Validated` / `Implemented` / `Foundation` / `Planned` /
  `Deferred` / `N/A`
- crate maturity: `Active` / `Stabilizing` / `Skeleton` / `Planned`
- milestones: `Completed` / `In Progress` / `Deferred`

Do not invent synonyms (`Done`, `Partial`, `Stable`), and do not mark a
milestone `Completed` without a green CI run to cite. Verifier/profile
definitions are the single source of truth for test case counts; documents
only copy them.
