// Minimal FreeRTOSConfig.h for OSAL native C shim smoke build.
// Intentionally sets OSAL_FREERTOS_TASK_TLS_INDEX beyond available slots
// to verify the compile-time _Static_assert in osal_freertos_shim.c.

#define configSUPPORT_DYNAMIC_ALLOCATION 1
#define INCLUDE_xTaskGetSchedulerState   1
#define configUSE_TIMERS                 0
#define configTICK_RATE_HZ               1000
#define configMAX_PRIORITIES              8
#define configMAX_TASK_NAME_LEN           16
#define INCLUDE_vTaskDelay                1
#define configNUMBER_OF_CORES             1
#define configUSE_MUTEXES                 1
#define INCLUDE_vTaskDelete                1
#define configNUM_THREAD_LOCAL_STORAGE_POINTERS 1
#define OSAL_FREERTOS_TASK_TLS_INDEX      1
#define configMINIMAL_STACK_SIZE           64
