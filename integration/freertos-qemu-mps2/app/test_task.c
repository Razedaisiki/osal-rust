/* test_task.c — deterministic native helper-task harness (P7G Step 4-0).
 *
 * Integration-only FreeRTOS task wrappers.  These let us create and
 * manage native helper tasks for testing OSAL managed objects without
 * depending on the OSAL Task implementation.
 *
 * Every helper task MUST end by calling osal_test_task_exit() (which
 * calls vTaskDelete(NULL)).  Returning from a FreeRTOS task entry is
 * undefined behaviour.
 */

#include "test_task.h"
#include "FreeRTOS.h"
#include "task.h"

int32_t osal_test_task_spawn(osal_test_task_entry_t entry,
                             void *context,
                             uint32_t stack_words,
                             uint32_t priority)
{
    TaskHandle_t handle = NULL;
    BaseType_t rc;

    if (entry == NULL || stack_words == 0U
        || priority >= (uint32_t)configMAX_PRIORITIES) {
        return OSAL_TEST_TASK_SPAWN_INVALID;
    }

    rc = xTaskCreate(entry,
                     "osal-helper",
                     (configSTACK_DEPTH_TYPE)stack_words,
                     context,
                     (UBaseType_t)priority,
                     &handle);

    if (rc == pdPASS) {
        return OSAL_TEST_TASK_SPAWN_OK;
    }
    if (rc == errCOULD_NOT_ALLOCATE_REQUIRED_MEMORY) {
        return OSAL_TEST_TASK_SPAWN_NO_MEMORY;
    }
    return OSAL_TEST_TASK_SPAWN_INTERNAL;
}

uint32_t osal_test_task_stack_hwm(void)
{
    return (uint32_t)uxTaskGetStackHighWaterMark(NULL);
}

void osal_test_scheduler_suspend(void)
{
    vTaskSuspendAll();
}

void osal_test_scheduler_resume(void)
{
    (void)xTaskResumeAll();
}

void osal_test_task_exit(void)
{
    vTaskDelete(NULL);

    /* vTaskDelete(NULL) never returns, but the compiler does not
     * see that.  Spin forever as a safe fallback. */
    for (;;) {
        __asm volatile ("wfi");
    }
}
