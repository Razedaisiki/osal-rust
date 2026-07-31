# ADR 0028: FreeRTOS Task Object Model

## Status

Accepted (2026-07-28)
Amended: 2026-07-30 for P7G integration contract alignment (TLS macro renamed).

## Context

The OSAL Task trait requires spawn, join (with timeout, repeatable, cached
result), current-task identity, and live-entry counting. FreeRTOS provides
`xTaskCreate` for task creation, `vTaskDelete` for self-deletion, TLS pointer
slots for per-task context, and Event Groups for multi-waiter event
notification.

POSIX uses `pthread_create` + `pthread_join` + `pthread_cond_t` for join,
with a `Running → Finished → Joining → Joined` state machine because
`pthread_join` is single-consumer. FreeRTOS has no equivalent of
`pthread_join` — the native `xTaskCreate` task runs independently and
self-deletes. The OSAL join semantic (multiple consumers, cached result)
must be built from FreeRTOS primitives.

## Decision

### 1. Completion via EventGroup with sticky bit

Each FreeRTOS task owns a native Event Group. A single completion bit
(`TASK_COMPLETED_BIT = 1`) is set atomically when the task entry returns:

```rust
const TASK_COMPLETED_BIT: u32 = 1;
```

Joiners wait on this bit with `clear_on_exit = false` and `wait_for_all = true`.
The bit is **never cleared** — it is a permanent terminal signal. This means:

- All currently-blocked joiners are woken when the bit is set.
- Future joiners call `xEventGroupWaitBits` and return immediately
  (bit is already set).
- No waiter-credit protocol is needed (unlike Queue).
- No `Joining` intermediate state is needed (unlike POSIX).

### 2. Simplified completion state machine

```
Running ──(task entry returns)──→ Finished(code)
```

The OSAL `ExitCode` is stored in an `AtomicU32` and published with
Release ordering before the EventGroup bit is set. Joiners check the
atomic first (fast path); only if `Running` do they wait on the EventGroup.

No `Joining` state is required because the EventGroup is multi-consumer
— unlike `pthread_join` which can only be called once.

### 3. TLS for `current()` identity

FreeRTOS provides `vTaskSetThreadLocalStoragePointer` and
`pvTaskGetThreadLocalStoragePointer` with a configurable number of
slots (`configNUM_THREAD_LOCAL_STORAGE_POINTERS`).

A dedicated TLS index is reserved via a build-time constant:

```c
#define OSAL_FREERTOS_TASK_TLS_INDEX 0
```

The C shim wraps the TLS access:

```c
void osal_freertos_task_set_current_context(void *ptr);
void *osal_freertos_task_get_current_context(void);
```

The backend stores a pointer to the `TaskIdentity` struct in this slot.
The identity is guaranteed to remain alive for the duration of entry
execution (held by `Arc<TaskIdentity>` in the trampoline).

Rules:
- Set on entry to `spawn()`, before the user entry function runs.
- Cleared before the user entry function's resources are dropped.
- Non-OSAL tasks (including the main thread) have no TLS value → `None`.

The TLS index must be validated at compile time:

```c
_Static_assert(
    configNUM_THREAD_LOCAL_STORAGE_POINTERS > OSAL_FREERTOS_TASK_TLS_INDEX,
    "OSAL_FREERTOS_TASK_TLS_INDEX exceeds available TLS slots"
);
```

### 4. `xTaskCreate` without retaining native handle

FreeRTOS `xTaskCreate` requires a non-NULL output pointer for the native
`TaskHandle_t`. The C shim passes a temporary local `TaskHandle_t` pointer,
reads the result, and immediately discards it. The backend does NOT store
the native handle — OSAL tasks self-delete (call `vTaskDelete(NULL)` at
the end of the trampoline), so a stored native handle would become dangling.

OSAL maintains its own stable `TaskHandle` (a monotonically-increasing
`NonZeroUsize`, matching the POSIX/Mock backends).

### 5. Generic trampoline

The task entry is a generic `FnOnce() + Send + 'static` closure boxed
into a `TaskStart<F>` payload:

```rust
struct TaskStart<F> {
    identity: Arc<TaskIdentity>,
    completion: Arc<TaskCompletion>,
    entry: Option<F>,
}

unsafe extern "C" fn task_trampoline<F>(parameter: *mut c_void)
where
    F: FnOnce() + Send + 'static,
{
    let mut start = Box::from_raw(parameter.cast::<TaskStart<F>>());

    // 1. Install TLS current identity.
    sys::task_set_current_context(
        Arc::as_ptr(&start.identity).cast_mut().cast()
    );

    // 2. Register live count.
    let live_token = LiveTaskToken::acquire();

    // 3. Execute the user entry.
    let entry = start.entry.take()
        .expect("FreeRTOS task entry already consumed");
    entry();

    // 4. Drop live token (count decremented).
    drop(live_token);

    // 5. Clear TLS identity.
    sys::task_set_current_context(core::ptr::null_mut());

    // 6. Publish exit code and Finished state.
    start.completion.publish(ExitCode::SUCCESS);

    // 7. Release task-owned Arcs.
    drop(start);

    // 8. Self-delete — never returns.
    sys::task_delete_current();
    unreachable!();
}
```

FreeRTOS tasks MUST NOT return from their entry function — they must
self-delete via `vTaskDelete(NULL)`. The `unreachable!()` after the
delete call enforces this at the Rust level.

### 6. Completion publish order

The order of operations when the task entry completes is fixed:

```
1. User entry returns
2. LiveTaskToken dropped (count decremented)
3. TLS current identity cleared
4. ExitCode written (Release store)
5. Finished state written (Release store)
6. EventGroup completion bit set (wakes all joiners)
7. Task-owned Arcs dropped
8. vTaskDelete(NULL) — never returns
```

This ordering guarantees that a joiner who observes `Finished` also observes:

- The user entry has completed.
- `count()` has been decremented.
- `current()` for the completed task returns `None`.
- The exit code is visible.

### 7. Join algorithm

```rust
fn join(&self, timeout: Timeout) -> Result<ExitCode> {
    // 1. Self-join guard.
    if Self::current() == Some(self.handle()) {
        return Err(Error::Busy);
    }

    // 2. Fast path — check cached state.
    if let Some(code) = self.completion.finished_code() {
        return Ok(code);  // no scheduler check needed
    }

    // 3. Non-blocking variants.
    match timeout {
        Timeout::NoWait | Timeout::After(Duration::ZERO) => {
            return Err(Error::Timeout);
        }
        _ => {}
    }

    // 4. Blocking — prepare budget (checks scheduler state, computes deadline).
    let mut budget = WaitBudget::new(timeout);
    budget.prepare_blocking()?;

    loop {
        // Re-check after wake.
        if let Some(code) = self.completion.finished_code() {
            return Ok(code);
        }

        match budget.wait_once(|ticks| {
            sys::event_group_wait_bits(
                &self.completion.event_group,
                TASK_COMPLETED_BIT,
                false,  // clear_on_exit
                true,   // wait_for_all
                ticks,
            )
        })? {
            WaitOutcome::Acquired => {
                // EventGroup bit is set — Finished must be visible.
                if let Some(code) = self.completion.finished_code() {
                    return Ok(code);
                }
                panic!("completion bit set before Finished state");
            }
            WaitOutcome::Unavailable => {
                // Timeout — check one last time.
                if let Some(code) = self.completion.finished_code() {
                    return Ok(code);
                }
                return Err(Error::Timeout);
            }
        }
    }
}
```

Join behavior matrix:

| State | Timeout | Result |
|-------|---------|--------|
| Finished | Any | Cached code immediately |
| Running | `NoWait` | `Timeout` |
| Running | `After(0)` | `Timeout` |
| Running | Finite | Complete or `Timeout` |
| Running | `Forever` | Wait until complete |
| Self-join | Any | `Busy` |
| Scheduler NotStarted + blocking | Finite/Forever | `NotInitialized` |
| Scheduler Suspended + blocking | Finite/Forever | `Busy` |

Already-finished tasks can be joined without the scheduler running
(they only read the cached atomic state).

### 8. Stack size conversion

Public API uses bytes; FreeRTOS `xTaskCreate` takes stack depth in words
(`StackType_t` units). The backend converts with checked arithmetic:

```rust
fn stack_bytes_to_words(
    bytes: usize,
    word_size: usize,
    minimal_words: usize,
    max_words: usize,
) -> Result<usize> {
    let rounded = bytes
        .checked_add(word_size - 1)
        .ok_or(Error::Overflow)?
        / word_size;

    let words = rounded.max(minimal_words);

    if words > max_words {
        return Err(Error::Overflow);
    }

    Ok(words)
}
```

- Rounds up to the next word boundary.
- Enforces `configMINIMAL_STACK_SIZE` as a floor.
- Rejects values exceeding the native stack-depth-type maximum.

### 9. Priority mapping

`priority()` returns the value the caller passed to `TaskBuilder::priority()`.
The native FreeRTOS scheduling priority is the requested value saturated to
`configMAX_PRIORITIES - 1`:

```rust
let native_priority = requested_priority.min(max_priorities - 1);
```

This is a backend mapping — the public value is not altered. If an
application requests priority 100 on a system with `configMAX_PRIORITIES = 8`,
the native scheduling priority is 7, but `priority()` still returns 100.

### 10. Name mapping

Public API accepts up to 31 UTF-8 bytes (validated by `validate_task_config`).
FreeRTOS `xTaskCreate` takes a `const char *` name for debugging, subject
to `configMAX_TASK_NAME_LEN`. The backend pre-truncates the name to
`min(31, configMAX_TASK_NAME_LEN - 1)` bytes in Rust before passing a
NUL-terminated buffer to the C shim.

OSAL does not provide a `name()` getter on `Task`, so native truncation
has no observable effect on the public API.

### 11. RuntimeLease lifecycle

`TaskIdentity` holds a `RuntimeLease<'static>`. The lease is acquired
during `spawn()` (before `xTaskCreate`) and held for the lifetime of
the `Task` handle. When the last `Arc<TaskIdentity>` is dropped, the
lease is released.

This is distinct from `Task::count()`, which tracks executing entries
(not handle references). A finished task whose handle is still alive
has `count() == 0` but `active_objects() == 1`.

### 12. Constructor and rollback

Constructor order (ADR 0019 §6):

```
1. Validate parameters (name, stack_size)
2. Convert stack bytes → words (checked arithmetic)
3. Map requested priority → native priority (saturation)
4. Prepare NUL-terminated name for xTaskCreate
5. Acquire RuntimeLease
6. Create EventGroup
7. Allocate OSAL TaskHandle (atomic increment)
8. Construct TaskIdentity Arc + TaskCompletion Arc
9. Box<TaskStart<F>> containing entry, identity, completion
10. Box::into_raw() → xTaskCreate()
11. On xTaskCreate failure:
    - Reclaim Box via Box::from_raw()
    - LiveTaskToken not yet registered (only in trampoline)
    - EventGroup deleted via its Drop
    - RuntimeLease released via TaskIdentity Drop
12. Return FreeRtosTask { identity, completion }
```

All shared state (identity, completion, entry) is fully constructed
before `xTaskCreate` because FreeRTOS may start executing the new
task before `xTaskCreate` returns to the caller.

### 13. Clone and Drop

`FreeRtosTask` holds two `Arc`s: `identity: Arc<TaskIdentity>` and
`completion: Arc<TaskCompletion>`. `Clone` increments both strong counts.

`TaskIdentity::drop()` runs when the last handle is released. This
decrements the active-object count (via `RuntimeLease` drop).

`TaskCompletion::drop()` runs at the same time. This deletes the
native EventGroup via `vEventGroupDelete`. The completion EventGroup
is guaranteed to be deletable: by the time the last handle is dropped,
the task entry has either completed (EventGroup bit is set and will
never be waited on again) or the application has abandoned the task
(no joiners remain).

### 14. `count()` model

`Task::count()` returns the number of OSAL tasks whose entry function
has not yet completed. Implemented as a backend-global `AtomicUsize`
incremented by `LiveTaskToken::acquire()` in the trampoline and
decremented by `LiveTaskToken::drop()` when the entry returns.

`spawn()` does NOT increment the count — only the trampoline does,
because `xTaskCreate` success does not guarantee the entry has
started executing.

### 15. Send + Sync

`FreeRtosTask` is `Send + Sync`. The `TaskIdentity` and
`TaskCompletion` types are `Send + Sync` (their contents are
atomic or owned by FreeRTOS kernel objects).

## Consequences

- EventGroup provides sticky multi-consumer completion without
  waiter-credit protocol complexity.
- Simplified state machine (no `Joining` state) reduces join code
  compared to POSIX.
- TLS slot reservation must be explicit in application configuration
  (`OSAL_FREERTOS_TASK_TLS_INDEX`).
- No native task handle stored — `vTaskDelete` and other task-control
  APIs cannot be used on OSAL tasks from the backend.
- `xTaskCreate` with `NULL` output handle means the backend has no
  way to suspend, resume, or query the native task after creation.
  These operations are deferred to a future extension that would
  require storing the native handle.
- Join timeout uses the P7D `WaitBudget` with absolute-deadline loop
  and per-chunk guard tick.
- Self-join detection via TLS comparison (same pattern as POSIX).
- Completion publish order is normative — must be preserved in
  implementation to avoid observable gaps between count, identity,
  and exit code.
