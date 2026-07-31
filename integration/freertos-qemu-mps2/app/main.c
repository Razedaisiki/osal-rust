/* main.c — OSAL FreeRTOS QEMU MPS2 Cortex-M3 boot test.
 *
 * Sequences:
 *   1. platform_init()
 *   2. console_init()
 *   3. Print OSAL_BOOT_BEGIN
 *   4. Create native boot task
 *   5. vTaskStartScheduler()
 *   6. Should never return — fail if it does
 */

#include "FreeRTOS.h"
#include "task.h"

#include "console.h"
#include "platform.h"
#include "qemu_exit.h"

/* ------------------------------------------------------------------ */
/* Boot task stack and priority.                                      */
/* ------------------------------------------------------------------ */
#define BOOT_TASK_STACK_WORDS  configMINIMAL_STACK_SIZE
#define BOOT_TASK_PRIORITY     (configMAX_PRIORITIES - 1)

/* ------------------------------------------------------------------ */
/* Forward declarations.                                              */
/* ------------------------------------------------------------------ */
static void boot_task(void *context);
static void boot_fail(const char *reason);

/* ------------------------------------------------------------------ */
/* main                                                               */
/* ------------------------------------------------------------------ */
int main(void)
{
    platform_init();
    console_init();

    console_write_line("OSAL_BOOT_BEGIN");

    BaseType_t created = xTaskCreate(
        boot_task,
        "osal-boot",
        BOOT_TASK_STACK_WORDS,
        NULL,
        BOOT_TASK_PRIORITY,
        NULL
    );

    if (created != pdPASS) {
        boot_fail("task-create");
    }

    vTaskStartScheduler();

    /* vTaskStartScheduler should never return on success. */
    boot_fail("scheduler-returned");

    /* boot_fail calls qemu_exit_failure which spins forever,
     * but the compiler does not see that. */
    return 1;
}

/* ------------------------------------------------------------------ */
/* Boot task — validate scheduler + tick.                             */
/* ------------------------------------------------------------------ */
static void boot_task(void *context)
{
    (void)context;

    /* 1. Scheduler must be Running. */
    if (xTaskGetSchedulerState() != taskSCHEDULER_RUNNING) {
        boot_fail("scheduler-state");
    }

    /* 2. Record tick, delay, verify tick advanced. */
    TickType_t before = xTaskGetTickCount();

    vTaskDelay(pdMS_TO_TICKS(10));

    TickType_t after = xTaskGetTickCount();

    if (after <= before) {
        boot_fail("tick-not-advanced");
    }

    /* 3. Success. */
    console_write_line(
        "OSAL_BOOT_PASS "
        "scheduler=running "
        "tick_advanced=true"
    );

    console_write_line("OSAL_BOOT_END status=pass");

    /* QEMU 4.2.1 does not reliably exit via semihosting on MPS2.
     * Spin here — the run script uses a timeout and the verifier
     * checks the UART output for the pass marker. */
    for (;;) {
        __asm__ volatile ("wfi");
    }
}

/* ------------------------------------------------------------------ */
/* Failure path — print marker and exit.                              */
/* ------------------------------------------------------------------ */
static void boot_fail(const char *reason)
{
    console_write("OSAL_BOOT_FAIL reason=");
    console_write_line(reason);
    qemu_exit_failure();
}
