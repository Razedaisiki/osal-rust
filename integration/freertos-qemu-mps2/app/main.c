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
/* Runtime-image sentinels — prove .data copy and .bss zero.         */
/* ------------------------------------------------------------------ */
static volatile uint32_t c_data_sentinel = 0x13579BDFU;
static volatile uint32_t c_bss_sentinel;

/* ------------------------------------------------------------------ */
/* Forward declarations.                                              */
/* ------------------------------------------------------------------ */
static void boot_task(void *context);
static void boot_fail(const char *reason);
static void boot_fail_u32(const char *reason, uint32_t code);
static void console_write_u32(uint32_t value);

/* Rust staticlib entry (P7G Step 3A).                                */
extern int32_t osal_rust_smoke_entry(void);

/* C shim delay — direct C call for diagnostic test A.               */
extern uint32_t osal_freertos_delay_ticks(uint64_t ticks);

/* ------------------------------------------------------------------ */
/* main                                                               */
/* ------------------------------------------------------------------ */
int main(void)
{
    platform_init();
    console_init();

    console_write_line("OSAL_BOOT_BEGIN");

    /* Validate runtime image initialisation before the scheduler.     */
    if (c_data_sentinel != 0x13579BDFU) {
        boot_fail("c-data-init");
    }
    if (c_bss_sentinel != 0U) {
        boot_fail("c-bss-init");
    }
    c_bss_sentinel = 0xA5A5A5A5U;
    if (c_bss_sentinel != 0xA5A5A5A5U) {
        boot_fail("c-bss-write");
    }

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

    /* 3. Diagnostic: direct C call to shim delay (test A).
     *    Emitted as OSAL_BOOT_DIAG — not required by verifier.       */
    {
        uint32_t diag_delay = osal_freertos_delay_ticks(2U);
        console_write("OSAL_BOOT_DIAG direct_delay_ticks=");
        console_write_u32(diag_delay);
        console_write_line("");
    }

    /* 4. Call into the Rust staticlib entry. */
    int32_t rust_code = osal_rust_smoke_entry();
    if (rust_code != 0) {
        boot_fail_u32("rust-entry", (uint32_t)rust_code);
    }

    /* 4. Success. */
    console_write_line(
        "OSAL_BOOT_PASS "
        "scheduler=running "
        "tick_advanced=true "
        "runtime_image=true "
        "rust_entry=true "
        "shim=true "
        "capabilities=true"
    );

    console_write_line("OSAL_BOOT_END status=pass");

    qemu_exit_success();

    /* If semihosting did not exit QEMU, spin as a safe fallback. */
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

/* ------------------------------------------------------------------ */
/* Write a u32 in decimal — no printf, no malloc.                     */
/* ------------------------------------------------------------------ */
static void console_write_u32(uint32_t value)
{
    char buf[12];
    int  i = 0;

    if (value == 0U) {
        console_write_byte('0');
        return;
    }

    while (value > 0U && i < (int)(sizeof(buf) - 1)) {
        buf[i++] = (char)('0' + (value % 10U));
        value /= 10U;
    }

    while (i > 0) {
        console_write_byte(buf[--i]);
    }
}

static void boot_fail_u32(const char *reason, uint32_t code)
{
    console_write("OSAL_BOOT_FAIL reason=");
    console_write(reason);
    console_write(" code=");
    console_write_u32(code);
    console_write_line("");
    qemu_exit_failure();
}
