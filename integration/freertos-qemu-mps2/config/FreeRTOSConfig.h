/*
 * FreeRTOSConfig.h — OSAL FreeRTOS QEMU MPS2 Cortex-M3 integration.
 *
 * Derived from the official FreeRTOS MPS2 demo FreeRTOSConfig.h
 * (FreeRTOS/FreeRTOS@592732b, FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/)
 * with the following intentional differences:
 *
 *   - configUSE_TIMERS=0            (OSAL uses own Timer Service Task)
 *   - configUSE_RECURSIVE_MUTEXES=0 (OSAL uses non-recursive mutexes only)
 *   - configSUPPORT_STATIC_ALLOCATION=0
 *   - configUSE_TRACE_FACILITY=0
 *   - configUSE_QUEUE_SETS=0
 *   - OSAL_FREERTOS_TASK_TLS_INDEX added
 *   - Configurations synced with ADR 0021/0028/0030
 *
 * Original FreeRTOSConfig.h from the MPS2 demo is MIT-licensed.
 */

#ifndef FREERTOS_CONFIG_H
#define FREERTOS_CONFIG_H

/*----------------------------------------------------------------------------*/
/* Scheduler configuration                                                    */
/*----------------------------------------------------------------------------*/
#define configNUMBER_OF_CORES                    1

#define configUSE_PREEMPTION                     1
#define configUSE_TIME_SLICING                   1
#define configIDLE_SHOULD_YIELD                  1

/* Tick and clock.  25 MHz CPU clock, 1 kHz tick.                             */
#define configCPU_CLOCK_HZ                       25000000UL
#define configTICK_RATE_HZ                       1000U
#define configTICK_TYPE_WIDTH_IN_BITS            TICK_TYPE_WIDTH_32_BITS

/* Task limits.                                                               */
#define configMAX_PRIORITIES                     8U
#define configMINIMAL_STACK_SIZE                 128U
#define configMAX_TASK_NAME_LEN                  16U
#define configSTACK_DEPTH_TYPE                   uint32_t

/* Allocation.                                                                */
#define configSUPPORT_DYNAMIC_ALLOCATION         1
#define configSUPPORT_STATIC_ALLOCATION          0
#define configTOTAL_HEAP_SIZE                    (128U * 1024U)

/* Synchronisation primitives.                                                */
#define configUSE_MUTEXES                        1
#define configUSE_RECURSIVE_MUTEXES              0
#define configUSE_COUNTING_SEMAPHORES            1
#define configUSE_QUEUE_SETS                     0

/* Timer — OSAL uses its own Timer Service Task (ADR 0029).                   */
#define configUSE_TIMERS                         0

#define configUSE_CO_ROUTINES                    0

/* Task notifications.                                                        */
#define configUSE_TASK_NOTIFICATIONS             1
#define configTASK_NOTIFICATION_ARRAY_ENTRIES    1U

/* TLS — OSAL Task identity (ADR 0028 §3).                                    */
#define configNUM_THREAD_LOCAL_STORAGE_POINTERS  1
#define OSAL_FREERTOS_TASK_TLS_INDEX             0

/* Included API functions.                                                    */
#define INCLUDE_xTaskGetSchedulerState           1
#define INCLUDE_vTaskDelay                       1
#define INCLUDE_vTaskDelete                      1
#define INCLUDE_vTaskSuspend                     0

/* Stack overflow checking.                                                   */
#define configCHECK_FOR_STACK_OVERFLOW           2

/* Hooks.                                                                     */
#define configUSE_MALLOC_FAILED_HOOK             1
#define configUSE_IDLE_HOOK                      0
#define configUSE_TICK_HOOK                      0

/* Cortex-M3 interrupt priorities (MPS2-AN385).                               */
#define configKERNEL_INTERRUPT_PRIORITY          255
#define configMAX_SYSCALL_INTERRUPT_PRIORITY     4

/* Port optimisations.                                                        */
#define configUSE_PORT_OPTIMISED_TASK_SELECTION  1
#define configENABLE_BACKWARD_COMPATIBILITY      0

/* assert — route FreeRTOS internal assertions to our fatal handler.           */
void platform_assert_failed(const char *file, unsigned long line);
#define configASSERT(x) \
    do { if ((x) == 0) { platform_assert_failed(__FILE__, __LINE__); } } while (0)

#endif /* FREERTOS_CONFIG_H */
