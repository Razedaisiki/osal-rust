// Minimal task.h for ROUSSATL native C shim smoke build.
// Provides the scheduler-state query that osal_freertos_shim.c uses.

#ifndef TASK_H
#define TASK_H

#include "FreeRTOS.h"

#define taskSCHEDULER_NOT_STARTED 1
#define taskSCHEDULER_RUNNING     2
#define taskSCHEDULER_SUSPENDED   0

BaseType_t xTaskGetSchedulerState(void);

#endif
