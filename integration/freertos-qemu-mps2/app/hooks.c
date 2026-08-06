/* hooks.c — FreeRTOS hook implementations for the OSAL boot test.
 *
 * These hooks intentionally avoid printf, malloc, and newlib buffering.
 * Every error path writes a distinct fatal marker and exits QEMU.
 */

#include "FreeRTOS.h"
#include "task.h"

#include "console.h"
#include "qemu_exit.h"
#include "test_task.h"

/* ------------------------------------------------------------------ */
/* Helpers                                                            */
/* ------------------------------------------------------------------ */

static void fatal(const char *kind, const char *detail)
{
    console_write("OSAL_BOOT_FATAL kind=");
    console_write(kind);
    if (detail != NULL) {
        console_write(" detail=");
        console_write(detail);
    }
    console_write_byte('\r');
    console_write_byte('\n');
    qemu_exit_failure();
}

/* ------------------------------------------------------------------ */
/* Expected-OOM fixture (P7G Step 4D)                                  */
/*                                                                     */
/* Allows exactly one pvPortMalloc failure from the controller task    */
/* during integration diagnostics, so that the real-kernel OOM test    */
/* can observe xTaskCreate returning errCOULD_NOT_ALLOCATE... rather   */
/* than the malloc-failed hook exiting QEMU.                           */
/* ------------------------------------------------------------------ */

#ifdef OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS

static TaskHandle_t expected_oom_owner;
static volatile uint32_t expected_oom_remaining;

void osal_test_expect_malloc_failure(void)
{
    expected_oom_owner = xTaskGetCurrentTaskHandle();
    expected_oom_remaining = 1;
}

uint32_t osal_test_expected_malloc_failure_consumed(void)
{
    return (expected_oom_remaining == 0) ? 1U : 0U;
}

void osal_test_clear_expected_malloc_failure(void)
{
    expected_oom_owner = NULL;
    expected_oom_remaining = 0;
}

#endif /* OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS */

/* ------------------------------------------------------------------ */
/* Hooks                                                              */
/* ------------------------------------------------------------------ */

void vApplicationMallocFailedHook(void)
{
#ifdef OSAL_FREERTOS_INTEGRATION_DIAGNOSTICS
    if (expected_oom_remaining == 1
        && xTaskGetCurrentTaskHandle() == expected_oom_owner)
    {
        expected_oom_remaining = 0;
        return;
    }
#endif

    fatal("malloc-failure", NULL);
}

void vApplicationStackOverflowHook(TaskHandle_t xTask,
                                   char *pcTaskName)
{
    (void)xTask;
    fatal("stack-overflow", pcTaskName);
}

/* ------------------------------------------------------------------ */
/* configASSERT — platform-level assertion failure.                   */
/* ------------------------------------------------------------------ */

void platform_assert_failed(const char *file,
                            unsigned long line)
{
    /* Write the file and line as separate bytes — no printf. */
    console_write("OSAL_BOOT_FATAL kind=config-assert file=");
    console_write(file);
    console_write(" line=");

    /* Simple integer → decimal output. */
    char buf[12];
    int  i = 0;
    unsigned long n = line;

    if (n == 0) {
        console_write_byte('0');
    } else {
        while (n > 0 && i < (int)(sizeof(buf) - 1)) {
            buf[i++] = (char)('0' + (n % 10));
            n /= 10;
        }
        while (i > 0) {
            console_write_byte(buf[--i]);
        }
    }

    console_write_byte('\r');
    console_write_byte('\n');
    qemu_exit_failure();
}
