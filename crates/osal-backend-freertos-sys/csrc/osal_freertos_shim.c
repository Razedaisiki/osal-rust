// osal_freertos_shim.c — C shim for ROUSSATL FreeRTOS backend
//
// This is the ONLY compilation unit that #includes FreeRTOS headers.
// It exposes a stable C ABI that the Rust -sys crate calls.

#include "osal_freertos_shim.h"

// ---------------------------------------------------------------------------
// Compile-time configuration checks
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

#ifndef configMAX_PRIORITIES
#error "FreeRTOSConfig.h must define configMAX_PRIORITIES"
#endif

#ifndef configMAX_TASK_NAME_LEN
#error "FreeRTOSConfig.h must define configMAX_TASK_NAME_LEN"
#endif

// ---------------------------------------------------------------------------
// FreeRTOS headers
// ---------------------------------------------------------------------------

#include "FreeRTOS.h"
#include "task.h"

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
    cap.scheduler_state    = (uint32_t) xTaskGetSchedulerState();
    return cap;
}

// ---------------------------------------------------------------------------
// Scheduler state
// ---------------------------------------------------------------------------

uint32_t osal_freertos_scheduler_state(void) {
    return (uint32_t) xTaskGetSchedulerState();
}
