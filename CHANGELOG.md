# Changelog

## P7G — FreeRTOS Real-Kernel Integration and Validation

### Step 2 — C-only Kernel Boot on QEMU Cortex-M3 — Completed

- ADR 0030: real-kernel validation platform (QEMU mps2-an385, Cortex-M3,
  FreeRTOS Kernel V11.3.0, ARM_CM3 port, heap_4.c).
- `third_party/freertos-kernel/` — FreeRTOS Kernel V11.3.0 submodule
  (9b777ae5c).
- `third_party/mps2-an385-reference/` — frozen vendor platform files
  (startup_gcc.c, CMSIS headers, linker script) with full provenance.
- `integration/freertos-qemu-mps2/` — independent C firmware with own
  `main.c`, `FreeRTOSConfig.h`, UART console, boot protocol, QEMU runner.
- C-only boot test validates scheduler start, SysTick tick advance,
  `vTaskDelay` wake, and structured UART output.
- GitHub Actions: `freertos-qemu-boot` job (build + QEMU + verify; CI #98 green).

**Verification scope:** Cortex-M3 QEMU, scheduler, SysTick, delay wake, UART protocol, semihosting exit.

### Step 3 — Rust Staticlib and Real C-Shim Integration — Completed

#### Step 3A — Rust staticlib linked into firmware
- Minimal `#![no_std]` `staticlib` crate (target `thumbv7m-none-eabi`,
  panic=abort, independent workspace).
- Single `extern "C"` entry `osal_rust_smoke_entry()` returning 0, called
  from C boot task after scheduler/tick validation.
- Proof: C → Rust ABI call from a real FreeRTOS task; `rust_entry=true`.

#### Step 3B — Runtime image init and real C-shim probe
- Project-owned Cortex-M3 startup (`.data` copy, `.bss` zero, HardFault
  with machine-parsable fatal marker).
- Rust `AtomicU32` data/bss sentinels validate runtime image init from
  within a FreeRTOS task.
- Real `osal-backend-freertos-sys` C shim linked (no `test-fixture`).
  `extern crate alloc` gated to `cfg(test-fixture)` in `-sys`.
- Cross-compiled with `arm-none-eabi-gcc` via `CC_thumbv7m_none_eabi`.
- Full smoke verified on real kernel:
  - `scheduler_state() == Running` (via `xTaskGetSchedulerState`)
  - All 11 capability fields (incl. `software_timers=false`, `tick_bits=32`)
  - `delay_ticks(2)` Rust → C shim → `vTaskDelay` → SysTick wake round-trip
  - Tick snapshot monotonic advance after delay
  - Requires 512-word task stack (128 words too small for dev-profile Rust)
- Protocol: `runtime_image=true`, `rust_entry=true`, `shim=true`,
  `capabilities=true`, `shim_delay=true`.
- GitHub Actions: builds Rust staticlib with `thumbv7m-none-eabi` target.
- QEMU exit via `bkpt #0xAB` (ARMv7-M Thumb semihosting).
- Host CI passes (0 failures).

**Not yet verified:** managed-object contracts (Mutex, Semaphore, Queue,
Task, Timer) on real kernel — deferred to Step 4.

### Step 3C — FreeRTOS Allocator and Runtime Lifecycle — Completed

- C shim: `osal_freertos_heap_alloc` / `osal_freertos_heap_dealloc`
  wrapping `pvPortMalloc` / `vPortFree` (P7G Step 3C Commit 1).
- Rust `GlobalAlloc`: over-allocation + header technique supporting
  arbitrary alignment.  All arithmetic checked.  OOM returns null.
- Allocator smoke: `Box<u32>`, `Box<Aligned64>` (align 64), `Arc`
  clone/drop/strong_count, `Vec<u32>` growth 0..128.  Exact heap
  recovery after all drops.
- Facade integration: `osal` with `backend-freertos` feature.
  Runtime lifecycle: 10 cases (initial state, pre-init rejection,
  initialize, AlreadyInitialized, Mutex create/lock/write/unlock,
  Busy shutdown, drop→shutdown, NotInitialized, reinitialize).
  8× init→mutex→shutdown cycle with exact heap recovery per cycle.
- Timer Service initializes (native mutex + semaphore + EventGroup)
  on first `initialize()`, releases on `shutdown()`.  Worker task
  remains lazy (not created).
- Task stack: 1024 words.  CI enforces a minimum remaining high-water
  mark of 128 words (Step 3C smoke observed ~459 words remaining).
- Heap evidence: baseline→alloc_live→after_alloc→after_init→
  with_mutex→after_shutdown — all exact recovery confirmed.
- Host CI: 0 failures.  QEMU exit code 0.

### Step 4 — Managed Object Real-Kernel Validation — In Progress

#### Step 4-0 — Deterministic native helper-task harness — Completed

**Integration-only infrastructure** for testing OSAL managed objects on
real FreeRTOS without depending on the OSAL Task implementation.

- `integration/freertos-qemu-mps2/app/test_task.h` / `test_task.c` —
  native FreeRTOS helper task API with parameter validation
  (entry==NULL, stack_words==0, priority>=configMAX_PRIORITIES).
  Functions: `osal_test_task_spawn`, `osal_test_task_stack_hwm`,
  `osal_test_scheduler_suspend`/`resume`, `osal_test_task_exit`
  (self-delete via `vTaskDelete(NULL)`).
  `OSAL_TEST_PHASE_*` C enum defines the phase constants.
- `integration/freertos-qemu-mps2/rust/src/harness.rs` — context-aware
  Rust harness:
  - `CaseState` with `AtomicU32` phase, visited bitmap, result,
    start_tick, end_tick.  Strictly monotonic phase transitions.
  - `wait_until_phase` with tick-bounded deadline, monotonic `>=` check,
    fail-fast on helper result (error surfaces as HelperResult, not
    masked as Timeout).
  - `wait_until_heap_recovered` polls with `delay_ticks(1)` to yield
    to Idle task for TCB/stack reclamation.
  - Context-aware extern "C" bridges: each native helper receives an
    opaque `*mut c_void` context pointing to its own `CaseState`,
    eliminating single-global cross-talk.  Proven by two simultaneous
    helpers with independent states.
- Full phase lifecycle: STARTED → BEFORE_OPERATION →
  OPERATION_COMPLETED → EXITING → DONE (controller sets DONE after
  confirming Idle cleanup).  Visited bitmap proves every phase was
  entered.
- Tick evidence: helpers record `xTaskGetTickCount` before/after
  `vTaskDelay(1)`; controller asserts `end - start >= 1`.
- Object protocol: `OSAL_OBJECT_BEGIN`, `OSAL_CASE_PASS name=<case>`,
  `OSAL_OBJECT_PASS`, `OSAL_OBJECT_END status=pass` — independent of
  boot protocol.
- Verifier: requires `harness_native_task` case, rejects unknown
  cases, missing/empty `name=`, duplicates; enforces CASE_PASS
  position between OBJECT_BEGIN and OBJECT_PASS; checks
  `multi_helper=true` and `tick_advance=true` in OBJECT_PASS.
- Boot protocol reordered to precede object protocol.
- Stack margin ~459 words (threshold 128).  QEMU exit code 0.
  All Step 3B/3C fields preserved.  `RUSTFLAGS="-D warnings"` clean.
- Context lifetime: `CaseState` instances are `static` so context
  pointers stay valid even when `run_harness_smoke` returns early on
  an error path while a spawned helper is still running.
- Phase ordering: `record_phase` enforces strict `current + 1`
  transitions within `CREATED..=DONE`; skipped, backward, and
  duplicate transitions set `RESULT_INVALID_PHASE` and are ignored.
  Range guard prevents shift overflow on bogus phase values from
  the C FFI bridge.

### Step 4A — Mutex Real-Kernel Contracts — Completed

- `cases/mutex.rs`: 8 Mutex contracts validated on real FreeRTOS.
- Infrastructure: `MutexTaskContext` with `Box::into_raw`/`Box::from_raw` pattern,
  `MutexOperation` enum (NoWait, AfterZero, AfterTicks,
  AfterTicksExpectAcquire, Forever), `SchedulerResumeGuard` (RAII).
- Rust native helper entry `mutex_helper_entry` — all MutexGuard
  drops complete before `vTaskDelete(NULL)`.

**Cases (all on real FreeRTOS Kernel V11.3.0, Cortex-M3, QEMU):**

| Case | What it proves |
|------|---------------|
| `mutex_basic_clone` | Clone is heap no-op; last-drop reclaims native handle |
| `mutex_non_recursive` | NoWait re-lock → LockFailed (same task) |
| `mutex_nowait_zero` | Cross-task NoWait→LockFailed, After(ZERO)→Timeout |
| `mutex_finite_timeout` | After(5ms)→Timeout, elapsed≥5 ticks |
| `mutex_blocking_wake` | Helper blocks, controller releases; acquired_tick≥release_tick |
| `mutex_forever_wake` | Forever acquires; finite watchdog; no spurious Timeout |
| `mutex_scheduler_suspended` | Suspended: After/Forever→Busy, NoWait/AfterZero non-blocking |
| `mutex_runtime_lease` | Active handle→Busy, failure-atomic; drop→shutdown→heap recovered |

- Suite tracks `suite_baseline` for end-to-end heap recovery proof.
  Final heap gate: `sys::heap_free() == suite_baseline` asserted after
  final shutdown, before `OSAL_OBJECT_PASS`.
- Per-helper TCB/stack recovery: each case records a `task_baseline`
  before spawn and waits for heap recovery after helper self-delete.
  `Box::into_raw` pattern for precise context lifetime control.
- `MutexTaskContext` owns a Mutex clone (not a raw pointer) — helper
  access remains valid even if the controller returns early on error.
  Spawn failure reclaims context immediately.
- `delay_ticks` return value checked in blocking cases.
- Stack margin: ~459 words (threshold 128). QEMU exit 0.
- Verifier: all 8 Mutex cases + harness case required; 6 Mutex
  OBJECT_PASS fields.

**Deferred (not in Step 4A scope):** fairness, starvation prevention,
priority inheritance, ISR calls, OOM injection, guard cross-task move.

### Step 4B — Semaphore Real-Kernel Contracts — Completed

- `cases/semaphore.rs`: 18 Semaphore contracts validated on real FreeRTOS
  (9 Counting + 7 Binary + 2 lifecycle/scheduler).
- Infrastructure: `CountingTaskContext` / `BinaryTaskContext` each own a
  semaphore clone (`Box::into_raw` pattern); `CountingOperation` /
  `BinaryOperation` enums; `counting_helper_entry` /
  `binary_helper_entry` Rust extern "C" native tasks.

**CountingSemaphore cases (all on real FreeRTOS Kernel V11.3.0):**

| Case | What it proves |
|------|---------------|
| `counting_core` | Invalid params rejected; max/count queries; acquire/release |
| `counting_overflow` | Release at max→Overflow; count unchanged (failure-atomic) |
| `counting_nowait_zero` | Empty: NoWait→Timeout, After(ZERO)→Timeout; available: both succeed |
| `counting_finite_timeout` | After(5ms)→Timeout, elapsed≥5 ticks, count unchanged |
| `counting_clone` | Clone heap no-op; last-drop reclaims native handle |
| `counting_blocking_wake` | Helper blocks, release wakes; completion_tick≥release_tick |
| `counting_forever_wake` | Forever acquires; controller watchdog; no spurious Timeout |
| `counting_one_release_one_waiter` | Two helpers, one release→exactly one completes |
| `counting_permit_accounting` | Two helpers, three releases→two wake+count=1 |

**BinarySemaphore cases:**

| Case | What it proves |
|------|---------------|
| `binary_core` | Unsig→release→sig→acquire→unsig |
| `binary_overflow` | Second release→Overflow, stays signaled |
| `binary_nowait_zero` | Unsig→Timeout; released→succeed+consume |
| `binary_blocking_wake` | Helper blocks→release wakes→signal consumed |
| `binary_forever_wake` | Forever acquire with controller watchdog |
| `binary_two_waiters` | One release→exactly one completes |
| `binary_clone` | Clone heap no-op; last-drop reclaims |

**Lifecycle + scheduler:**

| Case | What it proves |
|------|---------------|
| `semaphore_scheduler_suspended` | RAII guard; After/Forever→Busy; NoWait/AfterZero→Timeout |
| `semaphore_runtime_lease` | Active→Busy atomic; drop→shutdown→heap=suite_baseline |

- Per-helper TCB/stack recovery with `task_baseline` and `wait_until_heap_recovered`.
- Suite: init → Mutex → Semaphore → final shutdown → heap gate → OBJECT_PASS.
- Stack margin: ~459 words (threshold 128). QEMU exit 0.
- Verifier: 27 total cases required; 15 OBJECT_PASS fields.

**Deferred:** fairness, starvation prevention, ISR acquire/release,
OOM injection, high-concurrency stress.

### Step 4D — Task Real-Kernel Contracts — Completed

- `cases/task_contracts.rs`: 19 Task contracts + 1 harness case validated
  on real FreeRTOS Kernel V11.3.0 (Cortex-M3, QEMU mps2-an385).
- `TaskExitProbe`: unified Drop-based HWM recording for every spawned
  OSAL Task, replacing ad-hoc `task_stack_hwm()` calls.
- `DropProbe`: exact-once closure-capture teardown proof for OOM rollback.
- Join-wait diagnostics: C shim observers in `xEventGroupWaitBits` prove
  all concurrent joiners were simultaneously blocked.
- `TaskCaseState` with `Arc<AtomicU32>` for `stack_hwm`; `GateReleaseGuard<N>`
  RAII gate release for error-path cleanup.
- Expected-OOM fixture in `vApplicationMallocFailedHook`.
- Two-layer heap baseline: allocate test state → baseline → production ops →
  verify heap == baseline → drop test state → verify heap == global_baseline.

**Cases validated (profile: suite-task / PROFILE=task, 20 total):**

1 native harness case + 19 Task contract cases:

| Case | What it proves |
|------|---------------|
| `harness_native_task` | Native helper-task harness works |
| `task_builder_invalid` | Builder parameter validation (zero stack, overlong name, NUL) |
| `task_stack_mapping` | Stack bytes→words rounding + HWM >= 64 words |
| `task_priority_mapping` | Priority saturation to `configMAX_PRIORITIES-1` |
| `task_current_tls` | `Task::current()` returns Some(handle) inside OSAL task |
| `task_native_non_osal` | `Task::current()` returns None from native non-OSAL context |
| `task_live_count` | `Task::count()` reflects executing entries only |
| `task_nowait` | NoWait poll: Ok(ExitCode) if finished, Timeout otherwise |
| `task_after_zero` | After(ZERO): same as NoWait |
| `task_finite_timeout_retry` | Finite timeout → Timeout, retry succeeds |
| `task_join_forever_cached` | Forever join + cached code on repeated join |
| `task_self_join_busy` | Self-join returns Busy |
| `task_concurrent_joiners` | Three concurrent joiners all blocked in EventGroup wait, all get SUCCESS |
| `task_late_join_cached` | Late join under scheduler suspension reads cached code |
| `task_scheduler_suspended` | Suspended: blocking join → Busy |
| `task_drop_without_join` | Drop handle without join, task continues to completion |
| `task_finished_handle_lease` | Finished handle holds RuntimeLease, drop releases |
| `task_spawn_rollback` | Real xTaskCreate OOM: attempt+1/success+0, EventGroup create/delete once, closure DropProbe exactly once, Task count unchanged, heap exact recovery, recovery task proves runtime intact |
| `task_lifecycle_sequential` | 32 sequential spawn→join→drop rounds, per-task HWM |
| `task_lifecycle_concurrent` | 8 waves × 3 concurrent tasks, per-task HWM, self-delete + Idle cleanup |

**Sealing evidence:**

- `TaskExitProbe` on every task exit records HWM via `osal_test_task_stack_hwm()`
- `DropProbe` exact-once counter proves closure teardown on OOM rollback
- Join-wait diagnostics: `attempts_delta == 3` and `returns_delta == 0` before
  releasing target; all joiners require `ExitCode::SUCCESS`
- All 20 cases check `check_all_hwm()` (HWM >= 64 words per task)
- Final shutdown: profile heap == profile_baseline → OSAL_OBJECT_PASS

**Verification scope:** real FreeRTOS kernel under QEMU.
Physical MCU validation remains outstanding.

**Sealing baseline:** `c97581404f158abc6f7d99b91619b06b9552bf49`
**CI:** #133 — Success (9 jobs green, 3 QEMU artifacts)

### Step 4C-1 — Queue Core Real-Kernel Contracts — Completed

- `cases/queue.rs`: 9 Queue contracts validated on real FreeRTOS
  (all controller-side — no native helpers needed).
- Infrastructure: `QueueSendContext` / `QueueRecvContext` each own a
  Queue clone (`Box::into_raw` pattern); `SendOperation` / `RecvOperation`
  enums.

**Cases (all on real FreeRTOS Kernel V11.3.0, Cortex-M3, QEMU):**

| Case | What it proves |
|------|---------------|
| `queue_core_fifo` | Invalid params rejected; capacity/msg_size/len/empty/full; FIFO |
| `queue_wrong_size_precedence` | InvalidMessageSize priority over QueueClosed |
| `queue_nowait_zero` | NoWait→QueueFull/QueueEmpty, After(ZERO)→Timeout |
| `queue_clone_lifecycle` | Clone heap no-op; last-drop reclaims 3 native objects |
| `queue_recv_finite_timeout` | recv(After(5ms))→Timeout, elapsed≥5 ticks |
| `queue_send_finite_timeout` | send(After(5ms))→Timeout, elapsed≥5 ticks |
| `queue_close_drain` | Close→drain existing, further send/recv→QueueClosed, idempotent |
| `queue_scheduler_suspended` | After/Forever→Busy; NoWait/AfterZero non-blocking |
| `queue_runtime_lease` | Active→Busy atomic; drop→shutdown→heap=suite_baseline |

- configTOTAL_HEAP_SIZE: 128KB → 384KB (headroom for all suites).
- BOOT_TASK_STACK_WORDS: 1024 → 1280 (Queue ops in controller context).
- Stack margin: preserved above 128-word threshold. QEMU exit 0.
- Verifier: 36 total cases required; queue/queue_fifo/queue_timeout/
  queue_close/queue_suspended/queue_lease OBJECT_PASS fields.

**Deferred:** timeout/wake race, close/timeout priority (require
clock-control; deferred to Step 4C-3).

### Step 4C-2 — Queue Blocking Real-Kernel Contracts — Completed

- `cases/queue_blocking.rs`: 11 Queue blocking contracts validated
  in an **isolated FreeRTOS session** (separate Cargo feature
  `suite-queue-blocking`, independent QEMU run).
- Root cause of aggregate-suite helper creation failure identified:
  resource pressure from accumulated Mutex + Semaphore + Queue
  allocations, not a Queue code defect.  Isolated suite at 1024 words
  per helper works correctly.

**Cases (isolated profile, real FreeRTOS Kernel V11.3.0, Cortex-M3):**

| Case | What it proves |
|------|---------------|
| `queue_helper_resource_probe` | recv(After(5ms))→Timeout; stack HWM>=64; TCB/stack reclaimed |
| `queue_recv_blocking_wake` | Helper blocks, controller sends → acquires; completion≥send tick |
| `queue_send_blocking_wake` | Helper blocks, controller recvs → sends; completion≥recv tick |
| `queue_recv_forever_wake` | Forever acquires; no spurious Timeout; controller watchdog |
| `queue_send_forever_wake` | Forever send succeeds; controller watchdog |
| `queue_one_send_one_receiver` | Two receivers, one send→exactly one wakes |
| `queue_one_recv_one_sender` | Two senders, one recv→exactly one wakes |
| `queue_close_broadcast_receivers` | Three receivers, close→all QueueClosed |
| `queue_close_broadcast_senders` | Three senders, close→all QueueClosed; M0 drainable; close idempotent |
| `queue_throughput_cycle` | 64 interleaved send/recv, FIFO, exact heap recovery |

- Spawn diagnostics: `SPAWN_OK/INVALID/NO_MEMORY/INTERNAL` codes.
- Helper stack HWM recorded per task.
- Stack margin: 399 words (threshold 128) in queue-blocking profile.
- Aggregate suite (36 cases) no regression.
- Two QEMU profiles: `make` (aggregate) and `make CARGO_FEATURES=suite-queue-blocking`.

### Step 1 — Integration Contract Neutralization — Completed

- Integration identifiers neutralized: `ROUSSATL_FREERTOS_*` env vars →
  `OSAL_FREERTOS_*`; `ROUSSATL_FREERTOS_TASK_TLS_INDEX` →
  `OSAL_FREERTOS_TASK_TLS_INDEX`.
- TLS slot is now explicitly required (compile-time `#error` if missing);
  the previous default-to-slot-0 fallback is removed.
- `configUSE_TIMERS` changed from required to optional; native fixture
  smoke build uses `configUSE_TIMERS=0`.
- ADR 0021, 0022 amended: neutral identifiers, `configUSE_TIMERS` optional,
  `target_os = "freertos"` gate removed, handle types / error mapping /
  callback safety aligned with current implementation, `configMINIMAL_STACK_SIZE`
  and TLS non-negative checks added.
- ADR 0028: TLS macro renamed to `OSAL_FREERTOS_TASK_TLS_INDEX`.
- CI: added negative-compile tests (missing TLS, out-of-range TLS,
  negative TLS, zero minimal stack) with diagnostic verification.
- Task concurrency tests: RAII `TestGuard` with serialization mutex
  for safe parallel runs.

## P7F — FreeRTOS Timer Foundation — Completed

Custom Timer Service Task architecture with osal-portable::TimerState.
Lazy worker creation, binary semaphore wake, take-execute-restore
dispatch, deterministic Virtual-mode fixture bridge.

### Added

- ADR 0029: custom timer service model (rejects native FreeRTOS timers).
- `FreeRtosTimer`: full `Timer` trait implementation.
- Timer Service: native mutex registry, binary wake semaphore, completion
  EventGroup, lazy worker.
- Deterministic Virtual-mode fixture bridge with request/ack flush protocol.
- 6 shared Timer core contract cases.
- 5 deterministic controlled Timer contract cases.
- TimerState semantic tests: change_period, reset, fixed-rate, coalescing.
- Callback reentry tests: self-stop, self-reset, self-restart,
  self-change-period, cross-timer control, lock-free destruction.
- Drop/shutdown lifecycle race tests: in-flight last drop, shutdown-waits,
  self-shutdown Busy, suspended retry.
- Failure-atomic construction and rollback tests: worker-create, registry
  reserve, ID overflow, partial init, shutdown/reinit cycle.
- Scheduling tests: earliest-deadline ordering, overdue periodic vs.
  one-shot fairness, wake interruption, finite-chunk long-deadline wait.
- `InternalTaskHandle` / `NativeTaskHandle` in `-sys` crate for internal
  service tasks.
- ClockControl + `ControlledTimerFactory` for FreeRTOS.

### Changed

- Behavior contract §12: stale-expiration rewritten implementation-neutral.
- `configUSE_TIMERS == 1` requirement removed from C shim.
- `fixture::reset()`: task threads joined before sync map cleared.
- All tests use deterministic Virtual mode — no wall-clock sleeps.

### Deferred

- ISR Timer extensions → P7G+.
- Real FreeRTOS kernel tick-interrupt validation → P7G.

## P7E — FreeRTOS Task Foundation (2026-07-28) — Completed

### Added

- ADR 0028: FreeRTOS Task Object Model (EventGroup sticky completion,
  TLS identity, stack/priority mapping, self-delete trampoline).
- `FreeRtosTask`, `FreeRtosTaskBuilder`: full `Task` and `TaskBuilder`
  trait implementations via `xTaskCreate` + EventGroup.
- EventGroup completion: sticky `TASK_COMPLETED_BIT` set once on exit;
  multi-consumer without waiter-credit protocol.
- TLS current-task identity via `vTaskSetThreadLocalStoragePointer`
  at configurable `ROUSSATL_FREERTOS_TASK_TLS_INDEX`.
- Simplified `Running → Finished` state machine (no `Joining` state).
- Generic trampoline `task_trampoline<F>` with fixed completion publish
  order: entry → count → TLS → exit code → Finished → EventGroup →
  self-delete.
- `stack_bytes_to_words()`: checked byte→FreeRTOS stack-word conversion
  with rounding, minimum enforcement, native depth-type overflow detection.
- `map_native_priority()`: saturation to `configMAX_PRIORITIES-1`.
- `LiveTaskToken` RAII for `Task::count()`.
- Constructor order with full rollback on `xTaskCreate` failure
  (reclaim Box, drop Arcs; no count/RuntimeLease leak).
- `join()`: self-join detection (`Busy`), fast-path cached state,
  `WaitBudget` blocking with EventGroup `wait_bits`.
- Already-finished tasks can be joined without scheduler running.
- Host fixture: `std::thread::spawn` task simulation with `JoinHandle`
  tracking and drain-on-reset, EventGroup `Mutex<u32>`+`Condvar`,
  TLS via `thread_local!`, scheduler-state gating, fault injection,
  parameter recording.
- 17 TaskCoreContract cases passing via `FreeRtosTaskFactory`.
- 21 Task concurrency tests: join variants, timeout/retry, real
  self-join (Busy), two concurrent joiners, late joiner cached,
  drop-without-cancel, scheduler-state preconditions, shutdown
  lifecycle, stack/priority mapping, 50-cycle stress.
- Facade routing for `Task`/`TaskBuilder` under `backend-freertos`.
- Compile-time checks: `INCLUDE_vTaskDelete==1`, TLS slot availability.
- Native fixture headers: `event_groups.h`, updated `task.h` and
  `portmacro.h` with TLS/delete/stack-depth declarations.

### Changed

- Capability matrix: Task Foundation → Implemented (host-contract-verified).
- `KernelCapabilities` extended: `minimal_stack_depth_words`,
  `max_stack_depth_words`, `tls_pointer_slots`, `task_tls_index`.
- Crate docs: P7D → P7E.

### Fixed (stabilization)

- Queue state-mutex double acquisition after blocking reacquisition,
  which could deadlock racing send/receive operations.  The custom
  re-acquire loops were replaced with `lock_state()` which handles
  immediate + blocking acquisition in one step with no re-loop.
- Task fixture reset ordering: host task threads are now joined
  before native fixture maps (EventGroup, task entries) are cleared.
- Self-join test: replaced the ineffective test with a real in-task
  self-join via a shared slot (`Arc<Mutex<Option<FreeRtosTask>>>`).
- Added concurrent joiner coverage: two simultaneous joiners
  (Barrier), late joiner after task completion.
- Queue concurrency test stability: increased finite-timeout margins
  (10 ms → 50 ms); added `wait_until_blocked_count` before second
  wake in wake-one tests to prevent shared-Condvar lost wakeups.
- Fixed CI: `pdPASS`/`pdFAIL` in kernel portmacro, clippy
  `missing_const_for_thread_local` initializer, `unnecessary_cast`,
  `never_loop`, `needless_return`.

### Deferred to P7F+

- Task cancellation, suspend/resume.
- Real priority scheduling enforcement.
- Stack watermark, CPU affinity, SMP, MPU.
- Static task allocation.
- ISR task operations.
- Timer primitives.
- Real FreeRTOS kernel runtime tests for Validated promotion.

## P7D — FreeRTOS Queue Foundation (2026-07-27) — Completed

### Added

- ADR 0027: FreeRTOS Queue Object Model (ByteQueue + native mutex + dual
  wake semaphore, waiter-credit protocol, WaitBudget, close broadcast).
- `FreeRtosQueue`: bounded FIFO byte-message queue implementing the full
  `Queue` trait (ADR 0027).
- ByteQueue as sole data/close state (reused from `osal-portable`).
- Native FreeRTOS mutex for state serialisation; two counting
  semaphores (sender_wake, receiver_wake) for waiter signalling.
- Waiter-credit protocol: per-direction `0 <= wake_credits <= waiters`
  invariant prevents stale-token accumulation and semaphore overflow.
- `WaitBudget`: stateful absolute-deadline budget preserving a single
  deadline across repeated wait attempts within one API call.
- Close-drain broadcast: `close()` wakes all registered senders and
  receivers via missing-credit count; idempotent.
- `QueueStateGuard`: `!Send + !Sync` mutex guard wrapper.
- `NativeQueueResources`: RAII guard for constructor rollback.
- Constructor validates params and allocates `ByteQueue` before native
  objects (parameter errors don't require native-object rollback).
- Lock order: MUST NOT block on wake semaphore while holding state
  mutex (ADR 0027 §3).
- 2 Queue contract test functions running 21 shared contract cases
  (18 QueueCoreContract + 3 clone lifetime).
- 25 Queue concurrency tests: cross-thread wake, wake-one, timeout-race,
  close broadcast (receiver + sender), scheduler-state preconditions,
  multi-chunk finite, stress cycle.
- Facade routing for `Queue` under `backend-freertos`.
- `portmacro.h` for native FreeRTOS smoke build.
- Per-object blocked waiter tracking in fixture (`blocked_count` per
  mutex/semaphore entry).

### Changed

- Fixture: `notify_one` → `notify_all` to prevent lost wakeups across
  objects sharing the global Condvar; `sync_reset` clears poison flag
  to prevent cascading test failures.
- Wake semaphore `max_count` uses native max (UBaseType_t) instead of
  queue capacity — required for close broadcast with more waiters than
  capacity.
- `wait.rs`: added `WaitBudget` enum with `wait_once()` for multi-wait
  operations and `prepare_blocking()` preflight; existing
  `wait_native()` preserved as convenience wrapper.
- Capability matrix: Queue Core → Implemented, Queue Blocking →
  Implemented (host-contract-verified).
- Crate docs: P7C → P7D.

### Fixed (review cycle, round 1)

- WaitBudget deadline overflow restored to `Error::Overflow` (was panic).
- Blocking preflight (`prepare_blocking()`) validates scheduler state
  and computes deadline before waiter registration.
- Timeout-race token: when a wake token arrives between timeout and
  mutex re-acquisition, the waiter now loops back to re-check the queue
  instead of returning `Timeout` (preventing lost wakeups).
- Close broadcast: both sender and receiver directions are attempted
  before any fatal panic (committed-state fatal policy).
- Waiter/credit arithmetic uses `checked_add`/`checked_sub` with
  `expect()` instead of `saturating_sub`/ambiguous `if credits > 0`
  — invariant violations surface immediately.

### Fixed (review cycle, round 2)

- `WaitBudget::prepared` flag: after `prepare_blocking()` succeeds,
  `wait_once()` skips the scheduler-state check — a waiter registered
  after preflight can no longer hit `NotInitialized`/`Busy` in the
  blocking path.
- All remaining `saturating_sub` calls in waiter-credit paths replaced
  with `checked_sub().expect()`; waiter increments use `checked_add`.
- Added `close_wakes_blocked_receiver` test (symmetric to sender).
- Restored strict `BLOCKED_COUNT` assertion in fixture `sync_reset()`
  (keep poison recovery, reject stranded threads).

### Known limitation

- Multi-waiter close broadcast tests (close waking N>1 waiters
  simultaneously) are deferred as a host-fixture coverage limitation.
  The current shared-Condvar model is non-deterministic for simultaneous
  cross-object wake scenarios; the implementation code paths are
  identical to the single-waiter case.  Full coverage requires the
  per-object Condvar fixture refactor, which is tracked as deferred.
  Single-waiter close broadcast (both receiver and sender directions)
  is fully tested.

### Deferred

- Real FreeRTOS kernel runtime tests for Validated promotion.
- Per-object Condvar fixture.
- ISR queue variants (`IsrQueue`, `send_from_isr`, `recv_from_isr`).
- Task, Timer primitives (P7E+).

## P7C — FreeRTOS Mutex and Semaphore Foundation (2026-07-24) — Completed

### Added

- ADR 0025: FreeRTOS Blocking Wait Model (absolute-deadline loop,
  per-chunk guard tick, Forever via finite-chunk loop, scheduler-state
  preconditions).
- ADR 0026: FreeRTOS Synchronization Object Model (native handle
  ownership via `Arc`, lock order, guard Drop order, `Send + Sync`
  conditions, native delete constraints).
- `FreeRtosMutex<T>`: native priority-inheritance mutex with RAII
  `FreeRtosMutexGuard<'a, T>` (`!Send + !Sync` via `PhantomData<Rc<()>>`).
- `FreeRtosCountingSemaphore`, `FreeRtosBinarySemaphore`: native kernel
  semaphores with kernel count as sole source of truth.
- Unified blocking-wait engine (`wait.rs`): absolute-deadline loop with
  per-chunk guard tick, shared by Mutex + Semaphore.
- `MutexHandle`, `SemaphoreHandle`: opaque wrapper types in `-sys` crate.
- `TakeStatus`, `GiveStatus` enums mapping native C status codes to
  safe Rust.
- Host synchronization fixture (`sync_fixture.rs`): `std::sync::Mutex` +
  `Condvar` with `BLOCKED_COUNT` atomic for deterministic cross-thread
  waiter tests.
- `configUSE_MUTEXES` compile-time check in C shim.
- `osal_freertos_max_semaphore_count()` native range probe.
- 48 FreeRTOS tests: 6 cross-thread concurrent (Barrier + mpsc watchdog),
  14 sync stabilisation, contract suites for Mutex + Semaphore +
  Runtime + System + Clock.
- Facade routing for `Mutex<T>`, `CountingSemaphore`, `BinarySemaphore`
  under `backend-freertos`.

### Changed

- Capability matrix: Mutex/CountingSemaphore/BinarySemaphore → Implemented
  (host-contract-verified; real FreeRTOS kernel tests deferred).
- Architecture: FreeRTOS backend moved from Planned to Active.
- Crate docs: P7A → P7C status with Implementation vs Validation boundary
  documented.

### Deferred

- Real FreeRTOS kernel runtime tests (QEMU or physical MCU) needed for
  Validated promotion.
- Deterministic virtual-time fixture refactor (per-object Condvar).
- ISR semaphore and mutex variants.
- `RecursiveMutex`, static allocation, SMP, MPU.
- Queue, Task, Timer primitives (P7D+).

## P7B — FreeRTOS Tick/Time Model, Clock and System (2026-07-24) — Completed

### Added

- ADR 0023: FreeRTOS Tick and Time Model (coherent `vTaskSetTimeOutState`
  snapshot, `u128` expanded tick, per-chunk guard tick, chunked delay).
- ADR 0024: FreeRTOS System Mapping (`heap_free` via `xPortGetFreeHeapSize`,
  critical sections via `taskENTER_CRITICAL`/`taskEXIT_CRITICAL`,
  `!Send + !Sync` guard, `configNUMBER_OF_CORES==1` assertion).
- `osal-portable::tick_time`: checked `TickConfig`/`TickSnapshot` ↔
  `Duration` conversion, ceiling `duration_to_ticks_ceil`, `max_finite_ticks`,
  18 unit tests covering wrap/saturation/overflow.
- `FreeRtosClock`: `now()` via coherent tick+overflow snapshot, `delay()`
  with per-chunk guard tick and absolute-deadline chunking.
- `FreeRtosSystem`: `heap_free()` via `xPortGetFreeHeapSize`,
  `enter_critical()` with nesting support.
- `FreeRtosCriticalSectionGuard`: `!Send + !Sync` via `PhantomData<Rc<()>>`.
- C shim: `osal_freertos_tick_snapshot_t`, `delay_ticks`,
  `max_finite_delay_ticks`, `heap_free`, `enter_critical`, `exit_critical`.
- Native fixture headers: `TimeOut_t`, `vTaskSetTimeOutState`, `vTaskDelay`,
  `portMAX_DELAY`, `portable.h`.
- Fixture: `AtomicU64` tick/overflow counters with configurable tick bits
  for wrap simulation.
- 8 backend-specific Clock stabilisation tests, 14 sync stabilisation tests.

### Changed

- Capability matrix: Clock → Implemented, System → Validated, Runtime
  Lifecycle → Implemented.
- Per-chunk guard tick algorithm corrected from single global guard to
  per-native-call guard.

### Fixed

- Fixture tick wrap: modulo-based instead of saturating_add.
- Fixture `max_finite_delay_ticks`: configurable for multi-chunk testing.
- Native fixture `TimeOut_t` field order matches official FreeRTOS.

## P7A — FreeRTOS Integration Boundary and Backend Skeleton (2026-07-23) — Completed

### Added

- ADR 0020: FreeRTOS Integration Boundary (scheduler owned by BSP/app,
  backend is guest, `initialize()` does NOT start scheduler).
- ADR 0021: FreeRTOS Configuration Contract (required `FreeRTOSConfig.h`
  macros, C shim capability probe, compile-time `#error` enforcement).
- ADR 0022: FreeRTOS FFI Boundary (three-layer: C shim → `-sys` → backend,
  opaque handles, callback safety, platform `cfg` gate).
- `osal-backend-freertos-sys` crate: C shim (`osal_freertos_shim.c/.h`),
  build.rs with `ROUSSATL_FREERTOS_{KERNEL,CONFIG,PORT}_INCLUDE` env vars.
- `osal-backend-freertos` crate: runtime lifecycle (`initialize`/`shutdown`/
  `runtime_state`) with `spin::RwLock<Option<Capabilities>>` capability cache.
- Feature separation: `backend-freertos` (native) vs `freertos-test-fixture`
  (host CI).
- Native smoke build with minimal FreeRTOS header stubs in
  `tests/freertos-native-fixture/`.
- 5 invalid feature combination tests in CI.

### Changed

- Facade feature exclusivity guard extended to three backends.
- CI: FreeRTOS fixture compile, native facade smoke, feature-guard checks.

## P6D — POSIX Backend Conformance Closure (2026-07-22) — Completed

### Verified

- Confirmed that the POSIX backend implements the complete current
  non-deferred `osal-api` trait surface: Queue, Mutex,
  CountingSemaphore, BinarySemaphore, Clock, Timer, System, Task,
  and TaskBuilder.
- No `todo!()`, `unimplemented!()`, placeholder `panic!()`, or
  unconditional `Error::Unsupported` in any POSIX trait method.
- All trait methods have contract test coverage (shared contracts
  or backend-specific tests).
- POSIX backend tests, facade tests, and full workspace tests pass.
- Runtime lifecycle verified: init → create objects → shutdown →
  re-init cycle works correctly with active-object gating.

### Changed

- Updated capability matrix: POSIX column marked Validated for all
  current non-deferred capabilities.
- Updated README project status to reflect P6D completion.

### Deferred (unchanged)

- Advanced task controls (cancellation, suspend/resume, real priority
  scheduling, stack watermark).
- ISR extension traits.
- FreeRTOS backend.
- Production BSP implementation.

## P6C — Documentation Baseline Freeze (2026-07-22) — Completed

### Changed

- Reconciled README status with P6A/P6B implementation progress.
  Replaced "P0-P5 complete" with current P6B milestone and
  capability matrix using Validated/Implemented/Foundation/Deferred
  terminology.
- Defined documentation source-of-truth hierarchy: code >
  behavior-contract > ADRs > architecture > foundation slices >
  README > CHANGELOG.
- Aligned `architecture.md` runtime model and allocation
  description with `behavior-contract.md` §2. Removed `alloc`
  as a Cargo feature; clarified `std` is reserved for future
  host-only integrations.
- Split architecture diagrams into current implementation and
  target extension. Added crate maturity labels.
- Marked BSP, FreeRTOS, ISR extensions, and EventFlags as
  explicitly Deferred or Planned.
- Added `docs/documentation-policy.md` with update triggers,
  status terminology, and ADR rules.

### Fixed

- Semaphore constructor parameter validation now precedes runtime
  lease acquisition (ADR 0019 §6).
- Resolved rustdoc intra-doc links in runtime module documentation.

## P6B — Runtime Lifecycle (2026-07-21) — Completed

### Added

- ADR 0014: Backend and BSP Responsibility Boundary (semantic ownership
  vs primitive provider, composition rules, init/shutdown ordering).
- ADR 0015: Runtime Lifecycle (four-state cycle, transactional guards,
  failure-atomic hooks, re-initialisable after shutdown).
- ADR 0016: Linearizable Runtime Lease Accounting (single AtomicUsize
  packing state + count, supersedes ADR 0015 double-check algorithm).
- `RuntimeState` enum in `osal-api` (`Uninitialized`, `Initializing`,
  `Running`, `ShuttingDown`).
- `RuntimeLifecycle` in `osal-shared` with transactional `initialize`/
  `shutdown` guards and `RuntimeLease` double-count for object tracking.
- `Error::Busy` (runtime in use or lifecycle transition in progress).
- 22 RuntimeLifecycle unit tests (20 state-machine + 2 concurrency).
  38 total `osal-shared` unit tests.
- RuntimeLease-based active-object accounting.
- `osal-bsp` dependency on `osal-api` removed per ADR 0014.
- Updated architecture docs and behavior contract error table.

## P6A — Task Semantic Alignment (2026-07-20)

### Breaking changes

- **`Task::current()` returns `Option<TaskHandle>`** instead of `Handle`.
  `None` outside an OSAL-created task context (no more magic-zero sentinel).
- **`Task::handle()` returns `TaskHandle`** (`NonZeroUsize` wrapper) instead
  of bare `Handle`.
- **`count()` semantics changed**: counts live entry executions, not handle
  references. Completed tasks whose handle still exists are not counted.
- **Builder validation unified**: `validate_task_config()` in `osal-shared`.
  Empty name is valid; embedded NUL, >31 bytes, zero stack → `InvalidParameter`.
  Setters no longer silently clamp invalid values.
- **`Error::NotInitialized` removed from `join()`** documentation (API cannot
  produce an unstarted `Task`).

### Added

- `TaskHandle` type (`NonZeroUsize`, `Debug`, `Clone`, `Copy`, `Eq`, `Hash`).
- `LiveTaskToken` RAII guard: increments on entry start, decrements on return.
  Correctly rolls back on `pthread_create` failure or Mock entry panic.
- Backend-local `current()` identity via `thread_local!` (POSIX and Mock).
- 17 `TaskCoreContract` tests shared by both backends.
- POSIX concurrency tests: barrier-based three-task concurrency, concurrent join.
- POSIX `pthread_attr_setstacksize` with explicit error handling.
- Mock panic rollback tests (TLS and count restoration on unwind).
- Count-test serialisation lock in testkit.

### Fixed

- Trampoline order: `drop(live_token)` *before* `set_finished()` so NoWait
  pollers see the correct count immediately.
- POSIX `do_pthread_join`: non-consuming `&self` join preserves the pthread
  handle on failure for retry.
- NoWait no longer calls `pthread_join` (blocking); it returns cached code
  directly from `Finished` or `Joined` state.

### Changed

- Behavior contract §8 Task and test matrix fully rewritten.
- Mock no longer claims concurrent scheduling or suspend/resume.
- Architecture public types list includes `TaskHandle`.
- ADR 0013 records all design decisions.

## P5 — Task Foundation Slice (2026-07-07)

### Added

- Mock task implementation (synchronous execution in `spawn()`).
- POSIX pthread-based task implementation with completion-state machine
  (`Running → Finished → Joining → Joined`).
- Task smoke contract tests (5 tests) for both Mock and POSIX backends.
- POSIX task timeout join tests.
- Facade `Task` and `TaskBuilder` aliases.
- Backend-agnostic `task` facade example.

### Notes

- Task entry functions return `()`. Normal return maps to
  `ExitCode::SUCCESS`.
- `drop` on a `Task` handle does **not** cancel the task.
- Repeated `join()` returns the cached exit code immediately.
- POSIX timeout join uses `pthread_cond_timedwait` on backend
  completion state rather than non-portable `pthread_timedjoin_np`.
- Mock task execution is synchronous in this foundation slice
  (no mock scheduler).
- Priority is stored and reported; scheduling effect is
  backend-specific.

### Deferred

- Cancellation, suspend/resume, real priority scheduling, CPU
  affinity, stack watermark, deterministic mock scheduler,
  FreeRTOS task mapping.

## P4 — System Foundation Slice (2026-07-07)

### Added

- `MockSystem` with atomic nesting counter for critical sections.
- `PosixSystem` with process-local recursive `pthread_mutex_t`.
- `sys::recursive_mutex` wrapper (`PTHREAD_MUTEX_RECURSIVE`).
- `SystemFactory` in testkit; 5 system contract tests (heap_free,
  enter_critical, guard-drop re-entry, nesting, reverse drop order).
- `System` facade alias and `System` trait in `osal::prelude`.
- Backend-agnostic `system` facade example.
- `critical_depth_for_test()` helper exposed on Mock for stabilisation tests.

### Notes

- `heap_free()` returns `usize::MAX` for both Mock and POSIX backends.
  Real heap introspection is deferred to the BSP/resource phase.
- POSIX critical sections use a process-local recursive mutex
  (separate from the non-recursive `PosixMutex` used by `Mutex<T>`).
- Mock critical sections model nested entry/exit as an atomic counter
  for deterministic single-context tests.
- Critical sections support nesting; the outermost guard drop fully
  exits.

## P2 — Semaphore Foundation Slice (2026-07-06)

### Added

- `CountingSemaphore` trait with `acquire`/`release`/`max_count`/`count`.
- `BinarySemaphore` trait with `acquire`/`release`/`is_signaled`.
- ADR 0008: ISR Extension Model (ISR removed from core traits).
- `CountingSemaphoreState` portable state machine in `osal-portable`.
- `MockCountingSemaphore` (Rc) and `MockBinarySemaphore` (delegation).
- `PosixCountingSemaphore` (Arc, mutex+condvar, monotonic clock).
- `PosixBinarySemaphore` (delegates to PosixCountingSemaphore).
- 14 CountingCore + 9 BinaryCore contract tests.
- 8 POSIX blocking contract tests (generic over SemaphoreFactory).
- `mock_semaphore` and `posix_semaphore` examples.
- `docs/semaphore-foundation-slice.md`.

### Changed

- **ISR removed from core semaphore traits** (matching Queue P0).
- **`count()` returns `Result<u32>`** (matching Queue `len()`).
- **`is_acquired()` → `is_signaled() -> Result<bool>`**.
- `max_count()` is immutable cached value (no lock).
- Behavior contract §10 fully updated.

## P1.1 — Mutex Correctness Stabilization (2026-07-06)

### Changed (Breaking)

- **`Mutex<T>` is now non-recursive.** Re-locking while a guard is
  alive returns `Error::LockFailed`. The previous recursive + `DerefMut`
  combination was unsound (aliased `&mut T`). Recursive locking is
  deferred to a future `RecursiveMutex` type.

### Fixed

- **Memory safety**: Mock `MockMutexInner` no longer has `unsafe impl
  Send/Sync`. Only one guard can exist at a time.
- **Handle model**: POSIX `PosixMutexImpl<T>` now uses
  `Arc<PosixMutexInner<T>>` and implements `Clone` per ADR 0006.
- **Clock correctness**: POSIX `timed_lock` now uses monotonic clock
  (`clock_gettime(CLOCK_MONOTONIC)` + `try_lock` loop) instead of
  `pthread_mutex_timedlock` which may use `CLOCK_REALTIME`.
- **Sys mutex type**: `PTHREAD_MUTEX_RECURSIVE` → `PTHREAD_MUTEX_ERRORCHECK`.
- **Contract tests**: Removed recursive tests; added non-recursive tests
  (`no_second_guard`, `clone_shares_state`, `drop_clone_keeps_alive`).

### Added

- ADR 0007: Mutex Access Model (non-recursive, single guard).
- `monotonic_now_raw()`, `nanosleep()`, `timespec_ge()` helpers in
  `sys/time.rs`.

## P1 — Mutex Vertical Slice (2026-07-06)

### Added

- ADR 0006: Object Handle Model (strong typed handles, no global ID registry).
- `Mutex<T>` backend implementations:
  - `MockMutex<T>` (`Rc` + `UnsafeCell` + `Cell<usize>`, recursive).
  - `PosixMutexImpl<T>` (`PTHREAD_MUTEX_RECURSIVE`, `try_lock`, `timed_lock`).
- `MutexCoreContract`: 8 tests covering creation, lock/unlock, recursive,
  guard semantics.
- `MutexBlockingContract`: 3 cross-thread tests (POSIX only).
- `mock_mutex` and `posix_mutex` examples.
- `docs/mutex-foundation-slice.md` — architecture, components, deferred items.

### Changed

- `sys/mutex.rs`: switched from `PTHREAD_MUTEX_ERRORCHECK` to
  `PTHREAD_MUTEX_RECURSIVE`.
- `sys/mutex.rs`: added `try_lock()` and `timed_lock()` methods.
- `behavior-contract.md`: fixed POSIX table (Mutex<T> → RECURSIVE);
  added timeout table, error mapping, non-requirements.
- README Mutex row updated from "API only" to fully implemented.
- `object-lifetime.md`: added Guard concept and four-layer object model.

### Fixed

- `docs/queue-foundation-slice.md`: removed "POSIX Queue implementation"
  from Intentionally Deferred; updated contract test counts; updated
  Status to Complete.

## P0 — Queue Vertical Slice Stabilization (2026-07-03)

### Added

- ADRs: error precedence, queue close semantics, ISR API policy, query
  method policy, mock runtime model.
- `Error::Overflow` now covers `capacity * msg_size` overflow.
- Error precedence rules: `InvalidMessageSize` > `QueueClosed` >
  `QueueFull`/`QueueEmpty` > `Timeout` > `Internal`.
- `After(Duration::ZERO)` semantics: resource available → success,
  unavailable → `Error::Timeout`.
- `ByteQueue::is_closed()` public accessor.
- Contract tests split into `QueueCoreContract` and `QueueBlockingContract`.
- Error precedence contract tests.
- CI workflow: format, clippy, test, docs, feature guards.

### Changed

- **`Queue::close()`**: return type `()` → `Result<()>`.
- **`Queue::len()`**: return type `usize` → `Result<usize>`.
- **`Queue::is_empty()`**: return type `bool` → `Result<bool>`.
- **`Queue::is_full()`**: return type `bool` → `Result<bool>`.
- `Queue::capacity()` and `Queue::msg_size()` are now documented as
  non-fallible (fixed at construction).
- `ByteQueue::new()` uses `checked_mul` and `try_reserve_exact` instead
  of the `vec![]` macro; returns `Error::Overflow` on overflow and
  `Error::OutOfMemory` on allocation failure.
- `ByteQueue::try_send()`: error precedence changed — checks
  `InvalidMessageSize` before `QueueClosed`.
- POSIX `QueueInner`: removed duplicate `closed` flag; uses
  `ByteQueue.is_closed()` as sole source.
- POSIX `QueueInner`: cached `capacity` and `message_size` for
  lock-free access.
- POSIX `send()`/`recv()`: no longer double-lock for size validation.
- POSIX `close()`: checks `is_closed()` for idempotency.
- Behavior contract: feature names unified to `backend-posix` /
  `backend-mock`.
- Behavior contract: Semaphore `release()` at max returns
  `Error::Overflow` instead of `Error::InvalidParameter`.
- Contract tests: all error assertions use precise `matches!` with
  exact variant.

### Removed

- **`Queue::isr_send()`** and **`Queue::isr_recv()`** removed from
  core `Queue` trait. ISR operations deferred to future `IsrQueue`
  extension trait.
- ISR methods removed from MockQueue and PosixQueue.
- ISR contract tests (`run_isr_contracts`) removed.
- Behavior contract: ISR descriptions removed from Queue and Mutex
  sections.
- Behavior contract: Mutex `isr_lock()` removed from contract doc
  (never existed in trait).
