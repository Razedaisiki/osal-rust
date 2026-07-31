/* hooks.c — FreeRTOS hook implementations for the OSAL boot test.
 *
 * These hooks intentionally avoid printf, malloc, and newlib buffering.
 * Every error path writes a distinct fatal marker and exits QEMU.
 */

#include "FreeRTOS.h"
#include "task.h"

#include "console.h"
#include "qemu_exit.h"

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
/* Hooks                                                              */
/* ------------------------------------------------------------------ */

void vApplicationMallocFailedHook(void)
{
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

/* ------------------------------------------------------------------ */
/* Timer interrupt stubs — required by the vendor vector table.       */
/* The MPS2 QEMU machine has TIMER0 and TIMER1 peripherals; we do     */
/* not use them.  FreeRTOS uses SysTick for the system tick.          */
/* ------------------------------------------------------------------ */

void TIMER0_Handler(void)
{
    for (;;) {
        __asm__ volatile ("wfi");
    }
}

void TIMER1_Handler(void)
{
    for (;;) {
        __asm__ volatile ("wfi");
    }
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
