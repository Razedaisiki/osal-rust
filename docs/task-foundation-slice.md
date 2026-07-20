# Task Foundation Slice

## Status

Complete — Task is implemented across API, Mock, POSIX, contract
tests, and facade.

## Scope

The Task foundation slice provides:

- `TaskBuilder::new()` with name, stack size, and priority configuration
- `spawn()` — create and start a task
- `join(timeout)` — wait for task completion (NoWait, After, Forever)
- Repeated join returns cached `ExitCode`
- Non-zero `TaskHandle` per task
- `Task::priority()` query
- `Task::current()` — returns `Option<TaskHandle>` (`Some` inside
  OSAL task, `None` from main or non-OSAL thread)
- `Task::count()` — number of OSAL tasks whose entry function has
  not yet completed (live count, not handle count)
- Mock backend (synchronous execution, per-thread TLS for `current()`)
- POSIX backend (pthread-based, `thread_local!` TLS for `current()`)
- 17 TaskCoreContract tests shared by both backends
- Facade exposure through `osal::prelude::*`

## Non-goals

This slice does **not** provide:

- Cancellation
- Suspend / resume
- Real priority scheduling guarantees
- CPU affinity
- Stack watermark
- Deterministic mock scheduler
- FreeRTOS task mapping
- Global task registry / object table

## Join semantics

| Timeout | Behaviour |
|---------|-----------|
| `NoWait` | Poll: return `Ok(ExitCode)` if task already finished, `Err(Timeout)` otherwise |
| `After(d)` | Block up to `d`; return `Err(Timeout)` on expiry, task handle remains valid for retry |
| `Forever` | Block until task completion |

After the task exits, `join()` caches the `ExitCode`. All subsequent
`join()` calls (any timeout variant) return the cached code immediately
without blocking.

## Drop semantics

Dropping a `Task` handle does **not** cancel the task. The task
continues to run independently. This is analogous to `std::thread::JoinHandle`
— dropping releases the handle, not the thread.

## Entry function

The entry passed to `spawn()` executes exactly once. Normal return
(from `FnOnce()`) maps to `ExitCode::SUCCESS`. The entry type is
`FnOnce() + Send + 'static` — no user-defined exit codes in this
foundation slice.

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

## Contract tests

**TaskCoreContract** (17 tests, both Mock and POSIX):

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

**TaskConcurrencyContract** (POSIX only): three concurrent tasks with
barrier, NoWait-count timing, timeout retry, drop without cancel.

## Deferred

- Cancellation (`cancel()`, `kill()`)
- Suspend / resume
- Priority scheduling enforcement
- CPU affinity (`set_affinity`)
- Stack high-water mark
- `TaskState` runtime queries
- FreeRTOS task mapping
- Deterministic mock scheduler (cooperative yield model)
