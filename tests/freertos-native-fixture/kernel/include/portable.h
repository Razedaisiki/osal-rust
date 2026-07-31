// Minimal portable.h for OSAL native C shim smoke build.
// Declares the heap API that real FreeRTOS provides via port.c / heap_n.c.

#ifndef PORTABLE_H
#define PORTABLE_H

#include <stddef.h>

size_t xPortGetFreeHeapSize(void);

#endif
