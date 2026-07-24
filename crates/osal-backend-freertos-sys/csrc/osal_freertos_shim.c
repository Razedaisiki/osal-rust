// osal_freertos_shim.c — C shim for ROUSSATL FreeRTOS backend
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

#ifndef configUSE_TIMERS
#error "FreeRTOSConfig.h must define configUSE_TIMERS"
#endif
#if configUSE_TIMERS != 1
#error "OSAL FreeRTOS backend requires configUSE_TIMERS = 1"
#endif

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
#error "ROUSSATL FreeRTOS backend requires configUSE_MUTEXES == 1"
#endif

// ---------------------------------------------------------------------------
// Capability probe
// ---------------------------------------------------------------------------

osal_freertos_capability_t osal_freertos_probe_capabilities(void) {
    osal_freertos_capability_t cap;
    cap.tick_rate_hz      = (uint32_t) configTICK_RATE_HZ;
    cap.max_priorities    = (uint32_t) configMAX_PRIORITIES;
    cap.max_task_name_len = (uint32_t) configMAX_TASK_NAME_LEN;
    cap.tick_bits         = (uint8_t) (sizeof(TickType_t) * 8);
    cap.stack_word_size   = (uint8_t)  sizeof(StackType_t);
    cap.dynamic_allocation = 1;  // enforced by #error above
    cap.software_timers    = 1;  // enforced by #error above
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
