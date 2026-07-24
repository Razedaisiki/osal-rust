// Minimal semphr.h for ROUSSATL native C shim smoke build.
// Declares the FreeRTOS semaphore API that osal_freertos_shim.c uses.

#ifndef SEMPHR_H
#define SEMPHR_H

#include "FreeRTOS.h"

typedef void *SemaphoreHandle_t;

// Mutex
SemaphoreHandle_t xSemaphoreCreateMutex(void);

// Counting semaphore
SemaphoreHandle_t xSemaphoreCreateCounting(uint32_t uxMaxCount,
                                           uint32_t uxInitialCount);

// Binary semaphore
SemaphoreHandle_t xSemaphoreCreateBinary(void);

// Common operations
int32_t xSemaphoreTake(SemaphoreHandle_t xSemaphore, uint32_t xTicksToWait);
int32_t xSemaphoreGive(SemaphoreHandle_t xSemaphore);
uint32_t uxSemaphoreGetCount(SemaphoreHandle_t xSemaphore);
void vSemaphoreDelete(SemaphoreHandle_t xSemaphore);

#endif
