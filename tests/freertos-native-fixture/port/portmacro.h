// Minimal portmacro.h for ROUSSATL native C shim smoke build.
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

#endif
