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
- Opaque task `Handle`
- `Task::priority()` query
- `Task::current()` — handle of the calling task
- `Task::count()` — number of known tasks
- Mock backend (synchronous execution)
- POSIX backend (pthread-based)
- Shared smoke contract tests
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
background thread or scheduler. Join immediately returns the cached
`ExitCode::SUCCESS`. This model is sufficient for contract smoke tests.

## POSIX implementation

POSIX uses `pthread_create` to launch a real thread. The backend
maintains internal completion state with these transitions:

```
Running → Finished(code) → Joining → Joined(code)
```

- `pthread_join` is called **once** internally by the first joiner.
- Subsequent `join()` calls return the cached exit code.
- Timeout join is implemented through `pthread_cond_timedwait` on
  completion state, not through non-portable `pthread_timedjoin_np`.
- `handle()` returns a per-process incrementing ID.
- `current()` returns `0` for unknown (non-OSAL) threads.
- `count()` returns the number of live OSAL task handles.

## Contract tests

| # | Test | Principle |
|---|------|-----------|
| 1 | `task_count_is_callable` | `count()` returns without panicking |
| 2 | `current_returns_valid_handle` | `current()` returns without panicking |
| 3 | `spawn_runs_entry_once` | Entry function executes exactly once |
| 4 | `join_returns_after_task_exit` | `join(Forever)` succeeds after task completes |
| 5 | `join_after_exit_returns_immediately` | Repeated join after exit returns cached code |

## Deferred

- Cancellation (`cancel()`, `kill()`)
- Suspend / resume
- Priority scheduling enforcement
- CPU affinity (`set_affinity`)
- Stack high-water mark
- `TaskState` runtime queries
- FreeRTOS task mapping
- Deterministic mock scheduler (cooperative yield model)
