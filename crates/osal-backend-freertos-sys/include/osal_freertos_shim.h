// osal_freertos_shim.h — stable C ABI for ROUSSATL FreeRTOS backend
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
    uint32_t scheduler_state;    // from xTaskGetSchedulerState()
} osal_freertos_capability_t;

// ---------------------------------------------------------------------------
// Scheduler state constants (mirrors FreeRTOS task.h)
// ---------------------------------------------------------------------------

#define OSAL_FREERTOS_SCHEDULER_NOT_STARTED 0
#define OSAL_FREERTOS_SCHEDULER_RUNNING    1
#define OSAL_FREERTOS_SCHEDULER_SUSPENDED  2

// ---------------------------------------------------------------------------
// Capability probe
// ---------------------------------------------------------------------------

osal_freertos_capability_t osal_freertos_probe_capabilities(void);

// ---------------------------------------------------------------------------
// Scheduler state query
// ---------------------------------------------------------------------------

uint32_t osal_freertos_scheduler_state(void);

#ifdef __cplusplus
}
#endif

#endif // OSAL_FREERTOS_SHIM_H
