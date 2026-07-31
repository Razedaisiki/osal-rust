// Minimal portmacro.h for OSAL native C shim smoke build.
// Provides the base types that a real FreeRTOS port would define.
// Not a real port — only satisfies the compile-time requirements.

#ifndef PORTMACRO_H
#define PORTMACRO_H

#include <stdint.h>

// Base types — matches a typical 32-bit ARM Cortex-M port.
typedef int32_t  BaseType_t;
typedef uint32_t UBaseType_t;
typedef uint32_t TickType_t;
typedef uint32_t StackType_t;

// portMAX_DELAY: all-bits-set for TickType_t (0xFFFFFFFF for 32-bit).
#define portMAX_DELAY ((TickType_t)(~((TickType_t)0)))

// Stack depth type (ADR 0028 §8)
typedef uint32_t configSTACK_DEPTH_TYPE;

// Return values for FreeRTOS API functions (ADR 0028).
#define pdPASS ((BaseType_t)1)
#define pdFAIL ((BaseType_t)0)

// Boolean constants used by semaphore/mutex API.
#define pdTRUE  ((BaseType_t)1)
#define pdFALSE ((BaseType_t)0)

#endif
