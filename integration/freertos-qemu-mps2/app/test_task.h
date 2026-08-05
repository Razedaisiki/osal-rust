/* test_task.h — deterministic native helper-task harness (P7G Step 4-0).
 *
 * Integration-only C bridge for spawning FreeRTOS native helper tasks
 * that let us test OSAL managed objects without depending on the OSAL
 * Task implementation itself.
 *
 * All identifiers use the neutral osal_test_ prefix.
 */

#ifndef TEST_TASK_H
#define TEST_TASK_H

#include <stdint.h>

/* ------------------------------------------------------------------ */
/* Phase enum — keep in sync with rust/src/harness.rs.                */
/* ------------------------------------------------------------------ */
enum {
    OSAL_TEST_PHASE_CREATED            = 0,
    OSAL_TEST_PHASE_STARTED            = 1,
    OSAL_TEST_PHASE_BEFORE_OPERATION   = 2,
    OSAL_TEST_PHASE_OPERATION_COMPLETED = 3,
    OSAL_TEST_PHASE_EXITING            = 4,
    OSAL_TEST_PHASE_DONE               = 5,
};

/* ------------------------------------------------------------------ */
/* Entry point type                                                   */
/* ------------------------------------------------------------------ */
typedef void (*osal_test_task_entry_t)(void *context);

/* ------------------------------------------------------------------ */
/* Spawn status codes.                                                */
/* ------------------------------------------------------------------ */
enum {
    OSAL_TEST_TASK_SPAWN_OK        =  0,
    OSAL_TEST_TASK_SPAWN_INVALID   = -1,
    OSAL_TEST_TASK_SPAWN_NO_MEMORY = -2,
    OSAL_TEST_TASK_SPAWN_INTERNAL  = -3,
};

/* ------------------------------------------------------------------ */
/* Spawn a native FreeRTOS task.                                      */
/*                                                                    */
/* Returns OSAL_TEST_TASK_SPAWN_OK on success.                        */
/* Rejects entry==NULL, stack_words==0, priority>=configMAX_PRIORITIES*/
/* with OSAL_TEST_TASK_SPAWN_INVALID.                                 */
/* Returns OSAL_TEST_TASK_SPAWN_NO_MEMORY if xTaskCreate fails with   */
/* errCOULD_NOT_ALLOCATE_REQUIRED_MEMORY.                              */
/* ------------------------------------------------------------------ */
int32_t osal_test_task_spawn(osal_test_task_entry_t entry,
                             void *context,
                             uint32_t stack_words,
                             uint32_t priority);

/* ------------------------------------------------------------------ */
/* Return the calling task's stack high-water mark in words.           */
/* ------------------------------------------------------------------ */
uint32_t osal_test_task_stack_hwm(void);

/* ------------------------------------------------------------------ */
/* Suspend / resume the FreeRTOS scheduler.                            */
/* ------------------------------------------------------------------ */
void osal_test_scheduler_suspend(void);
void osal_test_scheduler_resume(void);

/* ------------------------------------------------------------------ */
/* Self-delete the calling native helper task.                         */
/*                                                                    */
/* Calls vTaskDelete(NULL) and spins forever.  Must only be called     */
/* from a native helper task, never from an OSAL Task.                 */
/* ------------------------------------------------------------------ */
__attribute__((noreturn))
void osal_test_task_exit(void);

/* ------------------------------------------------------------------ */
/* Context-aware harness bridges — called by native helper tasks.     */
/*                                                                    */
/* `context` is an opaque pointer to the Rust CaseState.              */
/* ------------------------------------------------------------------ */
void osal_test_harness_set_phase(void *context, uint32_t phase);
void osal_test_harness_set_result(void *context, int32_t result);
void osal_test_harness_record_start(void *context, uint32_t tick);
void osal_test_harness_record_end(void *context, uint32_t tick);

#endif /* TEST_TASK_H */
