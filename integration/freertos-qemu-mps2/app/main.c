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
#include "test_task.h"

/* ------------------------------------------------------------------ */
/* Boot task stack and priority.                                      */
/* ------------------------------------------------------------------ */
#define BOOT_TASK_STACK_WORDS       1024U
#define MIN_BOOT_STACK_MARGIN_WORDS  128U
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

/* Rust staticlib entry (allocator smoke, P7G Step 3C).               */
extern int32_t osal_rust_smoke_entry(void);

/* Rust object test entry (P7G Step 4).                                */
extern int32_t osal_test_object_entry(void);

/* ------------------------------------------------------------------ */
/* C bridges for the Rust integration crate.                          */
/* ------------------------------------------------------------------ */

__attribute__((noreturn))
void osal_test_rust_fatal(uint32_t reason)
{
    if (reason == 1U) {
        console_write_line("OSAL_BOOT_FATAL kind=rust-panic");
    } else {
        console_write_line("OSAL_BOOT_FATAL kind=unknown");
    }
    qemu_exit_failure();
    for (;;) { __asm volatile ("wfi"); }
}

/* Simple line output bridge — used by the Rust harness for the
 * object protocol markers (OSAL_OBJECT_BEGIN, OSAL_CASE_PASS, etc.). */
void osal_test_console_line(const char *line)
{
    console_write_line(line);
}

static void console_write_u64(uint64_t value)
{
    char buf[21];
    int i = 0;

    if (value == 0U) {
        console_write_byte('0');
        return;
    }

    while (value > 0U && i < (int)sizeof(buf)) {
        buf[i++] = (char)('0' + (value % 10U));
        value /= 10U;
    }

    while (i > 0) {
        console_write_byte(buf[--i]);
    }
}

void osal_test_trace_u64(const char *name, uint64_t value)
{
    console_write("OSAL_RUNTIME_INFO ");
    console_write(name);
    console_write("=");
    console_write_u64(value);
    console_write_line("");
}

/* ------------------------------------------------------------------ */
/* Native helper task — harness smoke (P7G Step 4-0).                 */
/*                                                                     */
/* Context-aware: `context` is an opaque pointer to the Rust           */
/* CaseState.  The entry is shared by both helper instances; each      */
/* receives its own context so states are independent.                 */
/*                                                                     */
/* Full phase lifecycle:                                                */
/*   STARTED → BEFORE_OPERATION → OPERATION_COMPLETED → EXITING        */
/*                                                                     */
/* Records real tick snapshots for tick-advance evidence.              */
/* ------------------------------------------------------------------ */
void harness_smoke_helper(void *context)
{
    uint32_t start_tick;
    uint32_t end_tick;

    osal_test_harness_set_phase(context, OSAL_TEST_PHASE_STARTED);

    /* Record start tick before the delay.                             */
    start_tick = (uint32_t)xTaskGetTickCount();
    osal_test_harness_record_start(context, start_tick);

    osal_test_harness_set_phase(context,
                                OSAL_TEST_PHASE_BEFORE_OPERATION);

    /* Advance at least one tick so the controller can observe a
     * non-zero tick delta.                                            */
    vTaskDelay(1U);

    /* Record end tick after the delay.                                */
    end_tick = (uint32_t)xTaskGetTickCount();
    osal_test_harness_record_end(context, end_tick);

    /* Validate scheduler state.  Report failure via result but
     * continue advancing phases — do not encode failure as a
     * skipped phase.                                                  */
    if (xTaskGetSchedulerState() != taskSCHEDULER_RUNNING) {
        osal_test_harness_set_result(context, -1);
    }

    osal_test_harness_set_phase(context,
                                OSAL_TEST_PHASE_OPERATION_COMPLETED);

    osal_test_harness_set_phase(context, OSAL_TEST_PHASE_EXITING);

    osal_test_task_exit();
}

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

    /* 3. High-water mark before entering Rust.                       */
    {
        uint32_t before_hwm = (uint32_t)uxTaskGetStackHighWaterMark(NULL);
        console_write("OSAL_BOOT_DIAG stack_hwm_before=");
        console_write_u32(before_hwm);
        console_write_line("");
    }

    /* 4. Call into the Rust staticlib entry (full C-shim smoke).     */
    int32_t rust_code = osal_rust_smoke_entry();
    if (rust_code != 0) {
        boot_fail_u32("rust-entry", (uint32_t)rust_code);
    }

    /* 5. Boot protocol complete — emit PASS/END before object tests.  */
    console_write_line(
        "OSAL_BOOT_PASS "
        "scheduler=running "
        "tick_advanced=true "
        "runtime_image=true "
        "rust_entry=true "
        "shim=true "
        "capabilities=true "
        "shim_delay=true "
        "allocator=true "
        "runtime_lifecycle=true "
        "runtime_lease=true "
        "mutex=true "
        "heap_recovered=true "
        "lifecycle_cycles=8"
    );
    console_write_line("OSAL_BOOT_END status=pass");

    /* 6. Run managed-object real-kernel validation (P7G Step 4).      */
    int32_t object_code = osal_test_object_entry();
    if (object_code != 0) {
        boot_fail_u32("object-entry", (uint32_t)object_code);
    }

    /* 7. High-water mark after all Rust work completes.              */
    {
        uint32_t after_hwm = (uint32_t)uxTaskGetStackHighWaterMark(NULL);
        console_write("OSAL_BOOT_DIAG stack_hwm_after=");
        console_write_u32(after_hwm);
        console_write_line("");

        if (after_hwm < MIN_BOOT_STACK_MARGIN_WORDS) {
            boot_fail("stack-margin");
        }
    }

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
