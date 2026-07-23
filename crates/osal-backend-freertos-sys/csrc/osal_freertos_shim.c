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
