// Minimal task.h for ROUSSATL native C shim smoke build.
// Provides the scheduler-state query that osal_freertos_shim.c uses.

#ifndef TASK_H
#define TASK_H

#include "FreeRTOS.h"

#define taskSCHEDULER_NOT_STARTED 1
#define taskSCHEDULER_RUNNING     2
#define taskSCHEDULER_SUSPENDED   0

BaseType_t xTaskGetSchedulerState(void);

// ---------------------------------------------------------------------------
// Time-out / delay support (ADR 0023)
// ---------------------------------------------------------------------------

// Minimal TimeOut_t struct — mirrors the official FreeRTOS definition:
//   typedef struct xTIME_OUT {
//       BaseType_t xOverflowCount;
//       TickType_t xTimeOnEntering;
//   } TimeOut_t;
typedef struct {
    BaseType_t xOverflowCount;
    TickType_t xTimeOnEntering;
} TimeOut_t;

void vTaskSetTimeOutState(TimeOut_t *pxTimeOut);
void vTaskDelay(TickType_t xTicksToDelay);

// Critical section macros (ADR 0024)
#define taskENTER_CRITICAL() do { /* raise mask */ } while (0)
#define taskEXIT_CRITICAL()  do { /* restore mask */ } while (0)

// Task creation / deletion (ADR 0028)
typedef void (*TaskFunction_t)(void *);
typedef void *TaskHandle_t;

BaseType_t xTaskCreate(TaskFunction_t pxTaskCode,
                       const char *pcName,
                       configSTACK_DEPTH_TYPE usStackDepth,
                       void *pvParameters,
                       UBaseType_t uxPriority,
                       TaskHandle_t *pxCreatedTask);
void vTaskDelete(TaskHandle_t xTaskToDelete);

// TLS support (ADR 0028 §3)
void vTaskSetThreadLocalStoragePointer(TaskHandle_t xTaskToSet,
                                       BaseType_t xIndex,
                                       void *pvValue);
void *pvTaskGetThreadLocalStoragePointer(TaskHandle_t xTaskToQuery,
                                         BaseType_t xIndex);

// Current task handle query (P7F — Timer Service self-shutdown detection)
TaskHandle_t xTaskGetCurrentTaskHandle(void);

#endif
