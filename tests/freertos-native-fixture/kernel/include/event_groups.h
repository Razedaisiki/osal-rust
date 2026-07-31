// Minimal event_groups.h for OSAL native C shim smoke build.
// Provides the EventGroup types and declarations that the C shim uses.

#ifndef EVENT_GROUPS_H
#define EVENT_GROUPS_H

#include "FreeRTOS.h"

typedef void * EventGroupHandle_t;
typedef uint32_t EventBits_t;

EventGroupHandle_t xEventGroupCreate(void);
EventBits_t xEventGroupSetBits(EventGroupHandle_t xEventGroup,
                               EventBits_t uxBitsToSet);
EventBits_t xEventGroupWaitBits(EventGroupHandle_t xEventGroup,
                                EventBits_t uxBitsToWaitFor,
                                BaseType_t xClearOnExit,
                                BaseType_t xWaitForAllBits,
                                TickType_t xTicksToWait);
void vEventGroupDelete(EventGroupHandle_t xEventGroup);

#endif
