# Task Foundation Slice

## Status

Complete — Task is implemented across API, Mock, POSIX, FreeRTOS,
contract tests, and facade.

## Scope

The Task foundation slice provides:

- `TaskBuilder::new()` with name, stack size, and priority configuration
- `spawn()` — create and start a task
- `join(timeout)` — wait for task completion (NoWait, After, Forever)
- Repeated join returns cached `ExitCode`
- Non-zero `TaskHandle` per task
- `Task::priority()` query
- `Task::current()` — returns `Option<TaskHandle>` (`Some` inside
  OSAL task, `None` from non-OSAL context)
- `Task::count()` — number of OSAL tasks whose entry function has
  not yet completed (live count, not handle count)
- Mock backend (synchronous execution, per-thread TLS for `current()`)
- POSIX backend (pthread-based, `thread_local!` TLS for `current()`)
- FreeRTOS backend (`xTaskCreate` + EventGroup sticky completion +
  TLS slot for `current()`; 17 shared core contract cases + 21
  concurrency/boundary tests)
- Facade exposure through `osal::prelude::*`

## Non-goals

This slice does **not** provide:

- Cancellation
- Suspend / resume
- Real priority scheduling guarantees
- CPU affinity
- Stack watermark
- Deterministic mock scheduler
- Global task registry / object table

## Architecture

```
                 osal (facade)
                     |
         +-----------+-----------+
         |           |            |
  osal-backend-posix |    osal-backend-freertos
         |           |            |
    PosixTask         |      FreeRtosTask
         |           |            |
    pthread_create    |      xTaskCreate
    pthread TLS       |      EventGroup completion
    Mutex+Condvar     |      FreeRTOS TLS slot
                      |      LiveTaskToken
                      |
               osal-backend-mock
                      |
                   MockTask
                      |
                 synchronous spawn
                 thread_local! TLS
```

## Components

| Layer | Type | Location |
|-------|------|----------|
| API | `Task` trait | `crates/osal-api/src/traits/task.rs` |
| API | `TaskBuilder` trait | `crates/osal-api/src/traits/task.rs` |
| API | `TaskHandle`, `ExitCode`, `Priority` | `crates/osal-api/src/types.rs` |
| Shared | `validate_task_config` | `crates/osal-shared/src/validation.rs` |
| Shared | `RuntimeLease` | `crates/osal-shared/src/runtime.rs` |
| POSIX | `PosixTask`, `PosixTaskBuilder` | `crates/osal-backend-posix/src/task.rs` |
| Mock | `MockTask`, `MockTaskBuilder` | `crates/osal-backend-mock/src/task.rs` |
| FreeRTOS | `FreeRtosTask`, `FreeRtosTaskBuilder` | `crates/osal-backend-freertos/src/task.rs` |
| FreeRTOS | `TaskIdentity`, `TaskCompletion` | `crates/osal-backend-freertos/src/task.rs` |
| FreeRTOS | `LiveTaskToken`, `stack_bytes_to_words` | `crates/osal-backend-freertos/src/task.rs` |
| FreeRTOS | `WaitBudget` (join timeout) | `crates/osal-backend-freertos/src/wait.rs` |
| Facade | `Task`, `TaskBuilder` alias | `crates/osal/src/backend.rs` |
| Testkit | Task core contracts | `crates/osal-testkit/src/contract/task.rs` |
| Testkit | TaskFactory trait | `crates/osal-testkit/src/factory/task.rs` |

## Join semantics

| Timeout | Behaviour |
|---------|-----------|
| `NoWait` | Poll: return `Ok(ExitCode)` if task already finished, `Err(Timeout)` otherwise |
| `After(d)` | Block up to `d`; return `Err(Timeout)` on expiry, task handle remains valid for retry |
| `Forever` | Block until task completion |
| Self-join | Returns `Error::Busy` |
| Finished task | Any timeout returns cached code immediately, no scheduler dependency |
| Scheduler preconditions (FreeRTOS) | Blocking join requires `SchedulerState::Running`; `NotStarted` → `NotInitialized`, `Suspended` → `Busy` |

## Concurrent joiners

All joiners (simultaneous or late) receive the same cached `ExitCode`.
The first joiner unblocks when the task completes; subsequent joiners
read the cached result without blocking (POSIX via `Joined` state,
FreeRTOS via sticky EventGroup bit).

## Drop semantics

Dropping a `Task` handle does **not** cancel the task. The task
continues to run independently. This is analogous to `std::thread::JoinHandle`
— dropping releases the handle, not the thread.

## Entry function

The entry passed to `spawn()` executes exactly once. Normal return
(from `FnOnce()`) maps to `ExitCode::SUCCESS`. The entry type is
`FnOnce() + Send + 'static` — no user-defined exit codes in this
foundation slice.

## Task lifetime (FreeRTOS)

- User's `FreeRtosTask` handle holds `Arc<TaskIdentity>` + `Arc<TaskCompletion>`.
- The trampoline holds its own `Arc` clones, keeping the identity and
  completion alive for the duration of entry execution.
- Dropping the last external handle does not cancel the task — the
  trampoline's `Arc` references keep it running.
- `TaskIdentity` carries a `RuntimeLease<'static>`.  `shutdown()` returns
  `Busy` while any `RuntimeLease` is alive (handle held or task running).
- `Task::count()` tracks executing entries only (via `LiveTaskToken` in
  the trampoline).  A finished task whose handle is still held has
  `count() == 0` but `active_objects() >= 1`.

## Mock implementation

Mock executes the task entry synchronously in `spawn()`. There is no
background thread or scheduler. A `thread_local!` slot provides
`current()` identity during entry execution. A `LiveTaskToken` RAII
guard manages the live count. Join immediately returns the cached
`ExitCode::SUCCESS`. This model is sufficient for all 17 core contract
tests.

## POSIX implementation

POSIX uses `pthread_create` with `pthread_attr_setstacksize` to launch
a real thread. The backend maintains internal completion state:

```
Running → Finished(code) → Joining → Joined(code)
```

- `pthread_join` is called **once** internally by the first blocking
  joiner. `NoWait` returns the cached code directly without calling
  `pthread_join`.
- Subsequent `join()` calls return the cached exit code.
- Timeout join is implemented through `pthread_cond_timedwait` on
  completion state, not through non-portable `pthread_timedjoin_np`.
- `handle()` returns a non-zero `TaskHandle`.
- `current()` returns `Some(TaskHandle)` via `thread_local!` TLS
  set in the trampoline.
- `count()` returns the number of entries that have not yet completed
  (managed by `LiveTaskToken` RAII).

## FreeRTOS implementation (ADR 0028)

FreeRTOS uses `xTaskCreate` with a generic trampoline, EventGroup sticky
completion, and FreeRTOS TLS for `current()`:

- **Completion**: A native EventGroup with sticky `TASK_COMPLETED_BIT` is
  set once on task exit.  All joiners (past, present, future) observe it
  without a waiter-credit protocol.  State machine: `Running → Finished`.
- **TLS**: `vTaskSetThreadLocalStoragePointer` at `ROUSSATL_FREERTOS_TASK_TLS_INDEX`
  provides per-task identity for `current()`.
- **Stack**: bytes→words checked conversion with rounding, minimum
  enforcement, and native `configSTACK_DEPTH_TYPE` overflow detection.
- **Priority**: `priority()` reports the requested value; native priority
  saturates to `configMAX_PRIORITIES - 1`.
- **Trampoline**: `unsafe extern "C" fn task_trampoline<F>` with fixed
  completion publish order: entry → count → TLS → exit code → Finished →
  EventGroup → self-delete.
- **Join**: self-join detection → fast-path atomic state → `WaitBudget`
  blocking on EventGroup `wait_bits` with `clear_on_exit = false`.
  Already-finished tasks can be joined without the scheduler running.
- **Rollback**: Constructor order with full rollback on `xTaskCreate`
  failure (reclaim Box, drop Arcs).

## Contract tests

**TaskCoreContract** (17 tests, Mock + POSIX + FreeRTOS):

| # | Test | Principle |
|---|------|-----------|
| 1 | `create_with_default_config` | Builder defaults compile and spawn |
| 2 | `accept_empty_name` | `""` is valid |
| 3 | `accept_max_length_name` | 31-byte name is valid |
| 4 | `reject_nul_in_name` | Embedded NUL → `Error::InvalidParameter` |
| 5 | `reject_overlong_name` | >31 bytes → `Error::InvalidParameter` |
| 6 | `reject_zero_stack` | `stack_size(0)` → `Error::InvalidParameter` |
| 7 | `positive_stack_size_succeeds` | `stack_size(8192)` spawns OK |
| 8 | `spawn_runs_entry_exactly_once` | `AtomicUsize` counter == 1 |
| 9 | `join_returns_after_task_exit` | `join(Forever)` succeeds |
| 10 | `repeated_join_returns_cached` | Cached code returned immediately |
| 11 | `handle_is_nonzero` | `TaskHandle::get() != 0` |
| 12 | `handle_is_unique` | Two tasks get different handles |
| 13 | `current_from_within_task` | `Some(handle)` inside entry |
| 14 | `current_from_main_is_none` | `None` from main thread |
| 15 | `priority_is_preserved` | Priority stored and returned as-is |
| 16 | `count_reflects_live_tasks` | count inside entry > baseline |
| 17 | `finished_task_not_in_count` | Completed handle alive, count at baseline |

**TaskConcurrencyContract** (POSIX): three concurrent tasks with
barrier, NoWait-count timing, timeout retry, drop without cancel.

**FreeRTOS concurrency tests** (21 tests): join NoWait/After(0)/
finite/Forever, timeout retry, repeated join cached, self-join
(`Busy`), two concurrent joiners, late joiner cached, drop without
cancel, finished join ignores scheduler state, blocking join
scheduler preconditions, shutdown lifecycle, stack bytes→words
verification, native priority saturation, invalid parameter
rejection, 50-cycle stress.

## Status per backend

| Backend | Task Core | Task Concurrency |
|---------|-----------|-----------------|
| Mock | Validated | Foundation (sync) |
| POSIX | Validated | Validated |
| FreeRTOS | Implemented | Implemented |

FreeRTOS promotion to Validated requires real FreeRTOS kernel
runtime tests (QEMU or physical MCU).

## Deferred

- Cancellation (`cancel()`, `kill()`)
- Suspend / resume
- Priority scheduling enforcement
- CPU affinity (`set_affinity`)
- Stack high-water mark
- `TaskState` runtime queries
- Deterministic mock scheduler (cooperative yield model)
- Timer (P7F+)
