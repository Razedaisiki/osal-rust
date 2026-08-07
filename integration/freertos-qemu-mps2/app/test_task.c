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
#include <string.h>

/* ------------------------------------------------------------------ */
/* Integration diagnostics counters (P7G Step 4D).                     */
/* ------------------------------------------------------------------ */

#ifdef OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS

static uint32_t diag_task_create_attempts;
static uint32_t diag_task_create_successes;
static uint32_t diag_event_group_creates;
static uint32_t diag_event_group_deletes;
static uint32_t diag_last_stack_words;
static uint32_t diag_last_native_priority;
static uint32_t diag_last_name_len;
static uint32_t diag_join_wait_attempts;
static uint32_t diag_join_wait_returns;

void osal_test_observe_task_create_attempt(const char *name,
                                           uint32_t stack_words,
                                           uint32_t priority)
{
    diag_task_create_attempts++;
    diag_last_stack_words = stack_words;
    diag_last_native_priority = priority;
    if (name != NULL) {
        size_t len = strlen(name);
        diag_last_name_len = (uint32_t)(len > 0xFFFFFFFFu ? 0xFFFFFFFFu : len);
    } else {
        diag_last_name_len = 0;
    }
}

void osal_test_observe_task_create_result(int success)
{
    if (success) {
        diag_task_create_successes++;
    }
}

void osal_test_observe_event_group_create(void)
{
    diag_event_group_creates++;
}

void osal_test_observe_event_group_delete(void)
{
    diag_event_group_deletes++;
}

void osal_test_observe_join_wait_attempt(void)
{
    diag_join_wait_attempts++;
}

void osal_test_observe_join_wait_return(void)
{
    diag_join_wait_returns++;
}

uint32_t osal_test_diag_task_create_attempts(void)
{
    return diag_task_create_attempts;
}

uint32_t osal_test_diag_task_create_successes(void)
{
    return diag_task_create_successes;
}

uint32_t osal_test_diag_event_group_creates(void)
{
    return diag_event_group_creates;
}

uint32_t osal_test_diag_event_group_deletes(void)
{
    return diag_event_group_deletes;
}

uint32_t osal_test_diag_last_stack_words(void)
{
    return diag_last_stack_words;
}

uint32_t osal_test_diag_last_native_priority(void)
{
    return diag_last_native_priority;
}

uint32_t osal_test_diag_last_name_len(void)
{
    return diag_last_name_len;
}

uint32_t osal_test_diag_join_wait_attempts(void)
{
    return diag_join_wait_attempts;
}

uint32_t osal_test_diag_join_wait_returns(void)
{
    return diag_join_wait_returns;
}

void osal_test_diag_reset(void)
{
    diag_task_create_attempts = 0;
    diag_task_create_successes = 0;
    diag_event_group_creates = 0;
    diag_event_group_deletes = 0;
    diag_last_stack_words = 0;
    diag_last_native_priority = 0;
    diag_last_name_len = 0;
    diag_join_wait_attempts = 0;
    diag_join_wait_returns = 0;
}

#endif /* OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS */

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
