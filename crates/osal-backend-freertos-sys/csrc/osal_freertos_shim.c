// osal_freertos_shim.c — C shim for OSAL FreeRTOS backend
//
// This is the ONLY compilation unit that #includes FreeRTOS headers.
// It exposes a stable C ABI that the Rust -sys crate calls.

#include "osal_freertos_shim.h"

// ---------------------------------------------------------------------------
// FreeRTOS headers — must come first so config macros are visible.
// FreeRTOSConfig.h is included by FreeRTOS.h from the application/BSP.
// ---------------------------------------------------------------------------

#include "FreeRTOS.h"
#include "task.h"
#include "portmacro.h"
#include "portable.h"
#include "semphr.h"
#include "event_groups.h"

// ---------------------------------------------------------------------------
// Compile-time configuration checks (ADR 0021)
// ---------------------------------------------------------------------------

#ifndef configSUPPORT_DYNAMIC_ALLOCATION
#error "FreeRTOSConfig.h must define configSUPPORT_DYNAMIC_ALLOCATION"
#endif
#if configSUPPORT_DYNAMIC_ALLOCATION != 1
#error "OSAL FreeRTOS backend requires configSUPPORT_DYNAMIC_ALLOCATION = 1"
#endif

#ifndef INCLUDE_xTaskGetSchedulerState
#error "FreeRTOSConfig.h must define INCLUDE_xTaskGetSchedulerState"
#endif
#if INCLUDE_xTaskGetSchedulerState != 1
#error "OSAL FreeRTOS backend requires INCLUDE_xTaskGetSchedulerState = 1"
#endif

#ifndef INCLUDE_vTaskDelay
#error "FreeRTOSConfig.h must define INCLUDE_vTaskDelay"
#endif
#if INCLUDE_vTaskDelay != 1
#error "OSAL FreeRTOS backend requires INCLUDE_vTaskDelay = 1"
#endif

// P7F: The OSAL backend provides its own Timer Service Task.  Native FreeRTOS
// software timers (timers.c, xTimerCreate, the timer daemon task) are
// not required.  configUSE_TIMERS is still probed for capability
// reporting but may be 0.

#ifndef configTICK_RATE_HZ
#error "FreeRTOSConfig.h must define configTICK_RATE_HZ"
#endif
_Static_assert(configTICK_RATE_HZ > 0,
               "configTICK_RATE_HZ must be greater than zero");

#ifndef configMAX_PRIORITIES
#error "FreeRTOSConfig.h must define configMAX_PRIORITIES"
#endif
_Static_assert(configMAX_PRIORITIES > 0,
               "configMAX_PRIORITIES must be greater than zero");

#ifndef configMAX_TASK_NAME_LEN
#error "FreeRTOSConfig.h must define configMAX_TASK_NAME_LEN"
#endif
_Static_assert(configMAX_TASK_NAME_LEN > 0,
               "configMAX_TASK_NAME_LEN must be greater than zero");

#ifndef configMINIMAL_STACK_SIZE
#error "FreeRTOSConfig.h must define configMINIMAL_STACK_SIZE"
#endif
_Static_assert(configMINIMAL_STACK_SIZE > 0,
               "configMINIMAL_STACK_SIZE must be greater than zero");

// P7B: single-core only (ADR 0024 §5)
#ifndef configNUMBER_OF_CORES
#error "FreeRTOSConfig.h must define configNUMBER_OF_CORES"
#endif
_Static_assert(configNUMBER_OF_CORES == 1,
               "P7B FreeRTOS backend requires configNUMBER_OF_CORES == 1");

// P7C: mutex support (ADR 0026 §7)
#ifndef configUSE_MUTEXES
#error "FreeRTOSConfig.h must define configUSE_MUTEXES"
#endif
#if configUSE_MUTEXES != 1
#error "OSAL FreeRTOS backend requires configUSE_MUTEXES == 1"
#endif

// P7E: task delete support (ADR 0028 §5)
#ifndef INCLUDE_vTaskDelete
#error "FreeRTOSConfig.h must define INCLUDE_vTaskDelete"
#endif
#if INCLUDE_vTaskDelete != 1
#error "OSAL FreeRTOS backend requires INCLUDE_vTaskDelete == 1"
#endif

// P7E: TLS pointer slots (ADR 0028 §3)
#ifndef OSAL_FREERTOS_TASK_TLS_INDEX
#error "FreeRTOSConfig.h must define OSAL_FREERTOS_TASK_TLS_INDEX"
#endif

#ifndef configNUM_THREAD_LOCAL_STORAGE_POINTERS
#error "FreeRTOSConfig.h must define configNUM_THREAD_LOCAL_STORAGE_POINTERS"
#endif
_Static_assert(
    OSAL_FREERTOS_TASK_TLS_INDEX >= 0,
    "OSAL_FREERTOS_TASK_TLS_INDEX must be non-negative"
);
_Static_assert(
    configNUM_THREAD_LOCAL_STORAGE_POINTERS > OSAL_FREERTOS_TASK_TLS_INDEX,
    "OSAL_FREERTOS_TASK_TLS_INDEX exceeds configNUM_THREAD_LOCAL_STORAGE_POINTERS"
);

// ---------------------------------------------------------------------------
// Capability probe
// ---------------------------------------------------------------------------

osal_freertos_capability_t osal_freertos_probe_capabilities(void) {
    osal_freertos_capability_t cap;
    cap.tick_rate_hz             = (uint32_t) configTICK_RATE_HZ;
    cap.max_priorities           = (uint32_t) configMAX_PRIORITIES;
    cap.max_task_name_len        = (uint32_t) configMAX_TASK_NAME_LEN;
    cap.tick_bits                = (uint8_t) (sizeof(TickType_t) * 8);
    cap.stack_word_size          = (uint8_t)  sizeof(StackType_t);
    cap.dynamic_allocation       = 1;  // enforced by #error above
#ifdef configUSE_TIMERS
    cap.software_timers          = (configUSE_TIMERS != 0) ? (uint8_t)1 : (uint8_t)0;
#else
    cap.software_timers          = 0;
#endif
    cap.minimal_stack_depth_words = (uint32_t) configMINIMAL_STACK_SIZE;
    {
        configSTACK_DEPTH_TYPE stack_max =
            (configSTACK_DEPTH_TYPE)(~((configSTACK_DEPTH_TYPE)0));
        if (stack_max > (configSTACK_DEPTH_TYPE)0xFFFFFFFFu) {
            cap.max_stack_depth_words = 0xFFFFFFFFu;
        } else {
            cap.max_stack_depth_words = (uint32_t)stack_max;
        }
    }
    cap.tls_pointer_slots         = (uint8_t) configNUM_THREAD_LOCAL_STORAGE_POINTERS;
    cap.task_tls_index            = (uint8_t) OSAL_FREERTOS_TASK_TLS_INDEX;
    cap.reserved[0] = 0;
    cap.reserved[1] = 0;
    return cap;
}

// ---------------------------------------------------------------------------
// Scheduler state
// ---------------------------------------------------------------------------

uint32_t osal_freertos_scheduler_state(void) {
    BaseType_t state = xTaskGetSchedulerState();

    // Translate FreeRTOS internal values to stable OSAL ABI values.
    // Rust must not depend on FreeRTOS macro numeric values (ADR 0022).
    if (state == taskSCHEDULER_NOT_STARTED) {
        return OSAL_FREERTOS_SCHEDULER_NOT_STARTED;
    }
    if (state == taskSCHEDULER_RUNNING) {
        return OSAL_FREERTOS_SCHEDULER_RUNNING;
    }
    if (state == taskSCHEDULER_SUSPENDED) {
        return OSAL_FREERTOS_SCHEDULER_SUSPENDED;
    }
    return OSAL_FREERTOS_SCHEDULER_UNKNOWN;
}

// ---------------------------------------------------------------------------
// Tick snapshot (ADR 0023 §1)
// ---------------------------------------------------------------------------

osal_freertos_tick_snapshot_t osal_freertos_tick_snapshot(void) {
    TimeOut_t native;
    osal_freertos_tick_snapshot_t result;

    vTaskSetTimeOutState(&native);

    result.overflow_count = (uint64_t)(UBaseType_t)native.xOverflowCount;
    result.tick_count     = (uint64_t)native.xTimeOnEntering;

    return result;
}

// ---------------------------------------------------------------------------
// Delay (ADR 0023 §5-6)
// ---------------------------------------------------------------------------

uint32_t osal_freertos_delay_ticks(uint64_t ticks) {
    // Zero ticks: return immediately (caller should handle this case).
    if (ticks == 0) {
        return OSAL_FREERTOS_DELAY_OK;
    }

    // Validate tick range — must not exceed portMAX_DELAY - 1.
    if (ticks > (uint64_t)(portMAX_DELAY - 1)) {
        return OSAL_FREERTOS_DELAY_INVALID_TICKS;
    }

    // Scheduler must be Running for non-zero delay.
    BaseType_t state = xTaskGetSchedulerState();
    if (state != taskSCHEDULER_RUNNING) {
        return OSAL_FREERTOS_DELAY_SCHEDULER_STOPPED;
    }

    vTaskDelay((TickType_t)ticks);
    return OSAL_FREERTOS_DELAY_OK;
}

uint64_t osal_freertos_max_finite_delay_ticks(void) {
    return (uint64_t)(portMAX_DELAY - 1);
}

// ---------------------------------------------------------------------------
// Heap (ADR 0024 §1)
// ---------------------------------------------------------------------------

uint64_t osal_freertos_heap_free(void) {
    return (uint64_t)xPortGetFreeHeapSize();
}

// ---------------------------------------------------------------------------
// Critical section (ADR 0024 §2)
// ---------------------------------------------------------------------------

void osal_freertos_enter_critical(void) {
    taskENTER_CRITICAL();
}

void osal_freertos_exit_critical(void) {
    taskEXIT_CRITICAL();
}

// ---------------------------------------------------------------------------
// Synchronisation object support (ADR 0026)
// ---------------------------------------------------------------------------

uint64_t osal_freertos_max_semaphore_count(void) {
    // UBaseType_t may be narrower than u64; return its maximum value.
    return (uint64_t)(UBaseType_t)(~((UBaseType_t)0));
}

// ---------------------------------------------------------------------------
// Mutex (ADR 0026)
// ---------------------------------------------------------------------------

osal_freertos_mutex_handle_t osal_freertos_mutex_create(void) {
    SemaphoreHandle_t h = xSemaphoreCreateMutex();
    return (osal_freertos_mutex_handle_t)h;
}

uint32_t osal_freertos_mutex_take(osal_freertos_mutex_handle_t handle,
                                  uint64_t ticks) {
    if (handle == NULL) {
        return OSAL_FREERTOS_TAKE_INVALID;
    }
    BaseType_t result = xSemaphoreTake((SemaphoreHandle_t)handle,
                                       (TickType_t)ticks);
    if (result == pdTRUE) {
        return OSAL_FREERTOS_TAKE_ACQUIRED;
    }
    return OSAL_FREERTOS_TAKE_TIMEOUT;
}

uint32_t osal_freertos_mutex_give(osal_freertos_mutex_handle_t handle) {
    if (handle == NULL) {
        return OSAL_FREERTOS_GIVE_INVALID;
    }
    BaseType_t result = xSemaphoreGive((SemaphoreHandle_t)handle);
    if (result == pdTRUE) {
        return OSAL_FREERTOS_GIVE_OK;
    }
    return OSAL_FREERTOS_GIVE_INVALID;
}

void osal_freertos_mutex_delete(osal_freertos_mutex_handle_t handle) {
    if (handle != NULL) {
        vSemaphoreDelete((SemaphoreHandle_t)handle);
    }
}

// ---------------------------------------------------------------------------
// Semaphore (ADR 0026)
// ---------------------------------------------------------------------------

osal_freertos_semaphore_handle_t
osal_freertos_counting_semaphore_create(uint32_t max_count,
                                        uint32_t initial_count) {
    SemaphoreHandle_t h = xSemaphoreCreateCounting(
        (UBaseType_t)max_count, (UBaseType_t)initial_count);
    return (osal_freertos_semaphore_handle_t)h;
}

osal_freertos_semaphore_handle_t
osal_freertos_binary_semaphore_create(void) {
    SemaphoreHandle_t h = xSemaphoreCreateBinary();
    return (osal_freertos_semaphore_handle_t)h;
}

uint32_t osal_freertos_semaphore_take(osal_freertos_semaphore_handle_t handle,
                                      uint64_t ticks) {
    if (handle == NULL) {
        return OSAL_FREERTOS_TAKE_INVALID;
    }
    BaseType_t result = xSemaphoreTake((SemaphoreHandle_t)handle,
                                       (TickType_t)ticks);
    if (result == pdTRUE) {
        return OSAL_FREERTOS_TAKE_ACQUIRED;
    }
    return OSAL_FREERTOS_TAKE_TIMEOUT;
}

uint32_t osal_freertos_semaphore_give(osal_freertos_semaphore_handle_t handle) {
    if (handle == NULL) {
        return OSAL_FREERTOS_GIVE_INVALID;
    }
    BaseType_t result = xSemaphoreGive((SemaphoreHandle_t)handle);
    if (result == pdTRUE) {
        return OSAL_FREERTOS_GIVE_OK;
    }
    // errQUEUE_FULL → no more room in counting semaphore.
    return OSAL_FREERTOS_GIVE_FULL;
}

uint64_t osal_freertos_semaphore_count(osal_freertos_semaphore_handle_t handle) {
    if (handle == NULL) {
        return 0;
    }
    return (uint64_t)uxSemaphoreGetCount((SemaphoreHandle_t)handle);
}

void osal_freertos_semaphore_delete(osal_freertos_semaphore_handle_t handle) {
    if (handle != NULL) {
        vSemaphoreDelete((SemaphoreHandle_t)handle);
    }
}

// ---------------------------------------------------------------------------
// EventGroup wrappers (ADR 0028 §1)
// ---------------------------------------------------------------------------

osal_freertos_event_group_handle_t osal_freertos_event_group_create(void) {
    EventGroupHandle_t h = xEventGroupCreate();
#ifdef OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS
    if (h != NULL) {
        osal_test_observe_event_group_create();
    }
#endif
    return (osal_freertos_event_group_handle_t)h;
}

uint32_t osal_freertos_event_group_set_bits(
    osal_freertos_event_group_handle_t handle,
    uint32_t bits)
{
    if (handle == NULL) {
        return OSAL_FREERTOS_EVENT_GROUP_INVALID;
    }
    (void)xEventGroupSetBits((EventGroupHandle_t)handle,
                             (EventBits_t)bits);
    // xEventGroupSetBits returns the new value; we ignore it.
    return OSAL_FREERTOS_EVENT_GROUP_OK;
}

uint32_t osal_freertos_event_group_wait_bits(
    osal_freertos_event_group_handle_t handle,
    uint32_t bits,
    uint8_t  clear_on_exit,
    uint8_t  wait_for_all,
    uint64_t ticks)
{
    EventBits_t result;
    if (handle == NULL) {
        return OSAL_FREERTOS_EVENT_GROUP_INVALID;
    }

#ifdef OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS
    osal_test_observe_join_wait_attempt();
#endif

    result = xEventGroupWaitBits(
        (EventGroupHandle_t)handle,
        (EventBits_t)bits,
        (BaseType_t)clear_on_exit,
        (BaseType_t)wait_for_all,
        (TickType_t)ticks);

#ifdef OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS
    osal_test_observe_join_wait_return();
#endif

    // xEventGroupWaitBits returns the EventBits value before the wait
    // (or before clear if clear_on_exit).  We only care whether the
    // requested bits were set.
    if ((result & (EventBits_t)bits) == (EventBits_t)bits) {
        return OSAL_FREERTOS_EVENT_GROUP_OK;
    }
    return OSAL_FREERTOS_EVENT_GROUP_TIMEOUT;
}

void osal_freertos_event_group_delete(
    osal_freertos_event_group_handle_t handle)
{
    if (handle != NULL) {
        vEventGroupDelete((EventGroupHandle_t)handle);
#ifdef OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS
        osal_test_observe_event_group_delete();
#endif
    }
}

// ---------------------------------------------------------------------------
// Task wrappers (ADR 0028 §4-5)
// ---------------------------------------------------------------------------

uint32_t osal_freertos_task_create(
    osal_freertos_task_entry_t entry,
    const char *name,
    uint32_t    stack_depth_words,
    void       *parameter,
    uint32_t    priority)
{
    BaseType_t result;
    TaskHandle_t native_handle = NULL;

    if (entry == NULL) {
        return OSAL_FREERTOS_TASK_CREATE_INVALID;
    }
    if (stack_depth_words == 0) {
        return OSAL_FREERTOS_TASK_CREATE_INVALID;
    }

#ifdef OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS
    osal_test_observe_task_create_attempt(name, stack_depth_words, priority);
#endif

    result = xTaskCreate(
        (TaskFunction_t)entry,
        name,
        (configSTACK_DEPTH_TYPE)stack_depth_words,
        parameter,
        (UBaseType_t)priority,
        &native_handle);

#ifdef OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS
    osal_test_observe_task_create_result(result == pdPASS);
#endif

    if (result != pdPASS) {
        return OSAL_FREERTOS_TASK_CREATE_OOM;
    }

    // The native handle is intentionally not stored — OSAL tasks
    // self-delete and the native handle would become dangling.
    // xTaskCreate with a non-NULL output handle parameter is required
    // by FreeRTOS (it does not accept NULL for the handle pointer),
    // but we discard the value immediately (ADR 0028 §4).
    (void)native_handle;

    return OSAL_FREERTOS_TASK_CREATE_OK;
}

void osal_freertos_task_delete_current(void) {
    vTaskDelete(NULL);
    // vTaskDelete never returns.
    for (;;) {}
}

void osal_freertos_task_set_current_context(void *ptr) {
    vTaskSetThreadLocalStoragePointer(
        NULL,
        OSAL_FREERTOS_TASK_TLS_INDEX,
        ptr);
}

void *osal_freertos_task_get_current_context(void) {
    return pvTaskGetThreadLocalStoragePointer(
        NULL,
        OSAL_FREERTOS_TASK_TLS_INDEX);
}

osal_freertos_task_handle_t osal_freertos_task_get_current_handle(void) {
    return (osal_freertos_task_handle_t)xTaskGetCurrentTaskHandle();
}

osal_freertos_task_handle_t osal_freertos_internal_task_create(
    osal_freertos_task_entry_t entry,
    const char *name,
    uint32_t    stack_depth_words,
    void       *parameter,
    uint32_t    priority)
{
    TaskHandle_t native_handle = NULL;
    BaseType_t  result;

    if (entry == NULL || stack_depth_words == 0) {
        return NULL;
    }

#ifdef OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS
    osal_test_observe_internal_task_create_attempt(name, stack_depth_words, priority);
#endif

    result = xTaskCreate(
        (TaskFunction_t)entry,
        name,
        (configSTACK_DEPTH_TYPE)stack_depth_words,
        parameter,
        (UBaseType_t)priority,
        &native_handle);

#ifdef OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS
    osal_test_observe_internal_task_create_result(result == pdPASS, native_handle);
#endif

    if (result != pdPASS) {
        return NULL;
    }

    return (osal_freertos_task_handle_t)native_handle;
}

// ---------------------------------------------------------------------------
// Native heap allocation bridge (P7G Step 3C)
// ---------------------------------------------------------------------------

void *osal_freertos_heap_alloc(size_t size)
{
    if (size == 0U) {
        return NULL;
    }

    return pvPortMalloc(size);
}

void osal_freertos_heap_dealloc(void *pointer)
{
    if (pointer != NULL) {
        vPortFree(pointer);
    }
}
