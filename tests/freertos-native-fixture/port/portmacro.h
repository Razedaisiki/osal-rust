// Minimal portmacro.h for OSAL native C shim smoke build.
// Defines the base types FreeRTOS.h expects.

#ifndef PORTMACRO_H
#define PORTMACRO_H

#include <stdint.h>

typedef uint32_t TickType_t;
typedef uint32_t StackType_t;
typedef int32_t  BaseType_t;
typedef uint32_t UBaseType_t;

#define pdTRUE  1
#define pdFALSE 0
#define pdPASS  1
#define pdFAIL  0

// portMAX_DELAY: max value of TickType_t (~0 cast).
// For 32-bit TickType_t on the native fixture.
#define portMAX_DELAY ((TickType_t)0xFFFFFFFFUL)

#endif
