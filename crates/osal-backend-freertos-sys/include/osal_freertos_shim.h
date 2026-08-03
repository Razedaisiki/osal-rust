// osal_freertos_shim.h — stable C ABI for OSAL FreeRTOS backend
//
// This header is the ONLY compilation unit that #includes FreeRTOS
// headers.  All FreeRTOS interaction from Rust goes through the
// functions and types declared here.

#ifndef OSAL_FREERTOS_SHIM_H
#define OSAL_FREERTOS_SHIM_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// ---------------------------------------------------------------------------
// Capability struct — populated at compile time from FreeRTOSConfig.h
// ---------------------------------------------------------------------------

typedef struct {
    uint32_t tick_rate_hz;
    uint32_t max_priorities;
    uint32_t max_task_name_len;
    uint8_t  tick_bits;          // sizeof(TickType_t) * 8
    uint8_t  stack_word_size;    // sizeof(StackType_t)
    uint8_t  dynamic_allocation; // configSUPPORT_DYNAMIC_ALLOCATION != 0
    uint8_t  software_timers;    // configUSE_TIMERS != 0
    // ---- Task support (ADR 0028) ----
    uint32_t minimal_stack_depth_words;  // configMINIMAL_STACK_SIZE
    uint32_t max_stack_depth_words;      // max value of stack-depth type
    uint8_t  tls_pointer_slots;          // configNUM_THREAD_LOCAL_STORAGE_POINTERS
    uint8_t  task_tls_index;             // OSAL_FREERTOS_TASK_TLS_INDEX
    uint8_t  reserved[2];                // align to 32-bit boundary
} osal_freertos_capability_t;

// ---------------------------------------------------------------------------
// Scheduler state constants (mirrors FreeRTOS task.h)
// ---------------------------------------------------------------------------

#define OSAL_FREERTOS_SCHEDULER_NOT_STARTED 0
#define OSAL_FREERTOS_SCHEDULER_RUNNING     1
#define OSAL_FREERTOS_SCHEDULER_SUSPENDED   2
#define OSAL_FREERTOS_SCHEDULER_UNKNOWN     0xFFFFFFFFu

// ---------------------------------------------------------------------------
// Capability probe
// ---------------------------------------------------------------------------

osal_freertos_capability_t osal_freertos_probe_capabilities(void);

// ---------------------------------------------------------------------------
// Tick snapshot — coherent tick + overflow count (ADR 0023 §1)
// ---------------------------------------------------------------------------

typedef struct {
    uint64_t overflow_count;
    uint64_t tick_count;
} osal_freertos_tick_snapshot_t;

// ---------------------------------------------------------------------------
// Delay status codes
// ---------------------------------------------------------------------------

#define OSAL_FREERTOS_DELAY_OK                0u
#define OSAL_FREERTOS_DELAY_INVALID_TICKS     1u
#define OSAL_FREERTOS_DELAY_SCHEDULER_STOPPED 2u

// ---------------------------------------------------------------------------
// Scheduler state query
// ---------------------------------------------------------------------------

uint32_t osal_freertos_scheduler_state(void);

// ---------------------------------------------------------------------------
// Tick and delay API (ADR 0023)
// ---------------------------------------------------------------------------

osal_freertos_tick_snapshot_t osal_freertos_tick_snapshot(void);
uint32_t osal_freertos_delay_ticks(uint64_t ticks);
uint64_t osal_freertos_max_finite_delay_ticks(void);

// ---------------------------------------------------------------------------
// Heap and critical-section API (ADR 0024)
// ---------------------------------------------------------------------------

uint64_t osal_freertos_heap_free(void);
void osal_freertos_enter_critical(void);
void osal_freertos_exit_critical(void);

// ---------------------------------------------------------------------------
// Semaphore range query (ADR 0026)
// ---------------------------------------------------------------------------

uint64_t osal_freertos_max_semaphore_count(void);

// ---------------------------------------------------------------------------
// Opaque handle types (ADR 0026 §1)
// ---------------------------------------------------------------------------

typedef void *osal_freertos_mutex_handle_t;
typedef void *osal_freertos_semaphore_handle_t;

// ---------------------------------------------------------------------------
// Take / Give status codes
// ---------------------------------------------------------------------------

#define OSAL_FREERTOS_TAKE_ACQUIRED  0u
#define OSAL_FREERTOS_TAKE_TIMEOUT   1u
#define OSAL_FREERTOS_TAKE_INVALID   2u

#define OSAL_FREERTOS_GIVE_OK        0u
#define OSAL_FREERTOS_GIVE_FULL      1u
#define OSAL_FREERTOS_GIVE_INVALID   2u

// ---------------------------------------------------------------------------
// Mutex API (ADR 0026)
// ---------------------------------------------------------------------------

osal_freertos_mutex_handle_t osal_freertos_mutex_create(void);
uint32_t osal_freertos_mutex_take(osal_freertos_mutex_handle_t handle,
                                  uint64_t ticks);
uint32_t osal_freertos_mutex_give(osal_freertos_mutex_handle_t handle);
void osal_freertos_mutex_delete(osal_freertos_mutex_handle_t handle);

// ---------------------------------------------------------------------------
// Semaphore API (ADR 0026)
// ---------------------------------------------------------------------------

osal_freertos_semaphore_handle_t
osal_freertos_counting_semaphore_create(uint32_t max_count,
                                        uint32_t initial_count);
osal_freertos_semaphore_handle_t osal_freertos_binary_semaphore_create(void);
uint32_t osal_freertos_semaphore_take(osal_freertos_semaphore_handle_t handle,
                                      uint64_t ticks);
uint32_t osal_freertos_semaphore_give(osal_freertos_semaphore_handle_t handle);
uint64_t osal_freertos_semaphore_count(osal_freertos_semaphore_handle_t handle);
void osal_freertos_semaphore_delete(osal_freertos_semaphore_handle_t handle);

// ---------------------------------------------------------------------------
// Opaque handle type for native task identity
// ---------------------------------------------------------------------------

typedef void *osal_freertos_task_handle_t;

// ---------------------------------------------------------------------------
// Opaque handle types for EventGroup (ADR 0028 §1)
// ---------------------------------------------------------------------------

typedef void *osal_freertos_event_group_handle_t;

// ---------------------------------------------------------------------------
// EventGroup status codes (ADR 0028 §1)
// ---------------------------------------------------------------------------

#define OSAL_FREERTOS_EVENT_GROUP_OK         0u
#define OSAL_FREERTOS_EVENT_GROUP_TIMEOUT    1u
#define OSAL_FREERTOS_EVENT_GROUP_INVALID    2u

// ---------------------------------------------------------------------------
// EventGroup API (ADR 0028)
// ---------------------------------------------------------------------------

osal_freertos_event_group_handle_t osal_freertos_event_group_create(void);
uint32_t osal_freertos_event_group_set_bits(
    osal_freertos_event_group_handle_t handle,
    uint32_t bits);
uint32_t osal_freertos_event_group_wait_bits(
    osal_freertos_event_group_handle_t handle,
    uint32_t bits,
    uint8_t  clear_on_exit,
    uint8_t  wait_for_all,
    uint64_t ticks);
void osal_freertos_event_group_delete(
    osal_freertos_event_group_handle_t handle);

// ---------------------------------------------------------------------------
// Task create status codes (ADR 0028 §4)
// ---------------------------------------------------------------------------

#define OSAL_FREERTOS_TASK_CREATE_OK          0u
#define OSAL_FREERTOS_TASK_CREATE_OOM         1u
#define OSAL_FREERTOS_TASK_CREATE_INVALID     2u

// ---------------------------------------------------------------------------
// Task entry type (ADR 0028 §5)
// ---------------------------------------------------------------------------

typedef void (*osal_freertos_task_entry_t)(void *parameter);

// ---------------------------------------------------------------------------
// Task API (ADR 0028)
// ---------------------------------------------------------------------------

uint32_t osal_freertos_task_create(
    osal_freertos_task_entry_t entry,
    const char *name,
    uint32_t    stack_depth_words,
    void       *parameter,
    uint32_t    priority);
void osal_freertos_task_delete_current(void);
void osal_freertos_task_set_current_context(void *ptr);
void *osal_freertos_task_get_current_context(void);
osal_freertos_task_handle_t osal_freertos_task_get_current_handle(void);
osal_freertos_task_handle_t osal_freertos_internal_task_create(
    osal_freertos_task_entry_t entry,
    const char *name,
    uint32_t    stack_depth_words,
    void       *parameter,
    uint32_t    priority);

// ---------------------------------------------------------------------------
// Native heap allocation bridge (P7G Step 3C)
//
// Wraps the FreeRTOS pvPortMalloc / vPortFree for the Rust global
// allocator.  These are low-level BSP/application callbacks — not
// part of the public OSAL API.
// ---------------------------------------------------------------------------

#include <stddef.h>

void *osal_freertos_heap_alloc(size_t size);
void  osal_freertos_heap_dealloc(void *pointer);

// ---------------------------------------------------------------------------
// Capability struct — extended for Task support (ADR 0028 §7, §9)

// New fields appended after the existing ones.  The struct remains
// ABI-compatible with earlier versions (new fields zero when compiled
// against older headers, detected by the shim via sizeof checks).
// ---------------------------------------------------------------------------

#ifdef __cplusplus
}
#endif

#endif // OSAL_FREERTOS_SHIM_H
