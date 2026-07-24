// Minimal FreeRTOSConfig.h for ROUSSATL native C shim smoke build.
// Not a real configuration — only satisfies the compile-time checks
// in osal_freertos_shim.c.

#define configSUPPORT_DYNAMIC_ALLOCATION 1
#define INCLUDE_xTaskGetSchedulerState   1
#define configUSE_TIMERS                 1
#define configTICK_RATE_HZ               1000
#define configMAX_PRIORITIES              8
#define configMAX_TASK_NAME_LEN           16
#define INCLUDE_vTaskDelay                1
#define configNUMBER_OF_CORES             1
#define configUSE_MUTEXES                 1
