# ADR 0013: Task Context and Live Count

## Status

Accepted (2026-07-07)

## Context

P5 delivered a working Task foundation, but several semantic gaps
remained:

- `current()` returned `0` (bare `usize`) as a sentinel for
  "no current task", which is fragile and ambiguous.
- `count()` was tied to `Task` handle lifecycle (`Drop`) rather
  than to the actual execution context. A finished task whose
  handle was still alive counted as "live", while the entry
  function had already returned.
- `spawn()` failures could leave stale `TASK_COUNT` increments.
- Builder validation was split across setter methods and `spawn()`,
  with some backends silently clamping invalid values.
- Mock had no way to report `current()` within the synchronously
  executed entry.
- POSIX had no thread-local awareness of which OSAL task was
  currently running.

The behavior contract also included requirements that the current
API surface cannot satisfy (e.g. "join an unstarted task") and
required Mock to pass concurrency/timeout tests beyond its
synchronous execution model.

## Decision

1. **`TaskHandle`**: introduce an opaque non-zero wrapper around
   `usize` (`NonZeroUsize`). `Task::handle()` returns `TaskHandle`.
   `Task::current()` returns `Option<TaskHandle>` — `None` outside
   an OSAL-created task context.

2. **Live count decoupled from handles**: `count()` reflects the
   number of OSAL tasks whose entry function has not yet completed.
   Finished tasks whose handle still exists are not counted. A
   `LiveTaskToken` RAII guard (backend-internal) manages increment
   on entry start and decrement on entry return.

3. **Unified builder validation**: `osal-shared::validation` provides
   `validate_task_config()` called at the top of `spawn()`. Setters
   store raw values without clamping. Zero stack size and over-long
   names are rejected explicitly.

4. **Backend-local TLS**: POSIX uses `thread_local! { Cell<Option<TaskHandle>> }`
   guarded by a `CurrentTaskGuard` set in the trampoline. Mock uses
   the same pattern so that synchronous entry execution sees the
   correct handle.

5. **Mock does not claim concurrency**: Mock passes `TaskCoreContract`
   only. Concurrency/timeout tests are POSIX-only. The documentation
   matrix no longer requires Mock to support multi-task concurrency.

6. **No global task registry**: `current()` and `count()` are
   sufficient for MVP diagnostics. A full task registry (enumeration,
   lookup, external control) is deferred until concrete use cases
   demand it.

7. **"Join unstarted task" removed**: the public API cannot produce
   a `Task` without a successful `spawn()`, so `Error::NotInitialized`
   is removed from `join()` documentation.

## Rationale

- `Option<TaskHandle>` is more idiomatic Rust than a magic-zero
  sentinel.
- Decoupling `count()` from handle `Drop` makes the semantics
  testable and predictable.
- `LiveTaskToken` with RAII ensures correct counting even when
  `pthread_create` fails or Mock entry panics.
- Unified validation prevents backends from diverging on error
  conditions.

## Consequences

- Breaking API change: `current()` returns `Option<TaskHandle>`,
  not `Handle`.
- All backends must implement TLS-based `current()`.
- `MockTaskInner::drop()` no longer touches `TASK_COUNT`.
- `PosixTaskInner::drop()` no longer touches `TASK_COUNT`.
- Behavior contract §8 and the test matrix must be updated.
- `osal::prelude` must re-export `TaskHandle`.
