// Minimal FreeRTOS.h for ROUSSATL native C shim smoke build.
// Includes the config and port headers, then defines the base types
// that the real FreeRTOS.h would provide.

#include "FreeRTOSConfig.h"
#include "portmacro.h"

#include <stddef.h>

// Memory allocation (minimal — real app provides heap implementation).
void *pvPortMalloc(size_t xWantedSize);
void vPortFree(void *pv);
