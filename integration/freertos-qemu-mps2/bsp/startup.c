/* startup.c — project-owned Cortex-M3 startup for OSAL QEMU MPS2.
 *
 * Derived from the MPS2 QEMU FreeRTOS demo startup_gcc.c:
 *   Repository: https://github.com/FreeRTOS/FreeRTOS
 *   Commit:     592732b4d8e8da21f122322de4c421f89e0b4d18
 *   Path:       FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/build/gcc/startup_gcc.c
 *   License:    MIT
 *
 * Changes from the vendor original:
 *   - Rust-like .data copy and .bss zero in Reset_Handler before main()
 *   - HardFault emits OSAL_BOOT_FATAL kind=hard-fault then calls
 *     qemu_exit_failure()
 *   - main() return is a fatal error
 *   - Unused IRQ vectors point to Default_Handler (infinite WFI)
 */

#include <stdint.h>

/* ------------------------------------------------------------------ */
/* Linker-provided symbols                                            */
/* ------------------------------------------------------------------ */
extern uint32_t _sidata;   /* .data load address in FLASH              */
extern uint32_t _sdata;    /* .data start in RAM                       */
extern uint32_t _edata;    /* .data end in RAM                         */
extern uint32_t _sbss;     /* .bss start in RAM                        */
extern uint32_t _ebss;     /* .bss end in RAM                          */

/* Provided by the application.                                       */
extern int main(void);

/* Provided by bsp/qemu_exit.c.                                       */
extern void qemu_exit_failure(void);

/* FreeRTOS interrupt handlers.                                       */
extern void vPortSVCHandler(void);
extern void xPortPendSVHandler(void);
extern void xPortSysTickHandler(void);

/* ------------------------------------------------------------------ */
/* Default handler — infinite WFI                                     */
/* ------------------------------------------------------------------ */
static void __attribute__((naked)) Default_Handler(void)
{
    __asm volatile (
        "1:  wfi    \n"
        "    b   1b \n"
    );
}

/* ------------------------------------------------------------------ */
/* TIMER stubs — required by the vendor vector table.                 */
/* ------------------------------------------------------------------ */
extern void TIMER0_Handler(void);
extern void TIMER1_Handler(void);

void TIMER0_Handler(void) {
    for (;;) { __asm volatile ("wfi"); }
}
void TIMER1_Handler(void) {
    for (;;) { __asm volatile ("wfi"); }
}

/* ------------------------------------------------------------------ */
/* HardFault — machine-parsable fatal marker then QEMU exit.          */
/* ------------------------------------------------------------------ */
void HardFault_Handler(void)
{
    /* Minimal UART write — no printf, no allocation.                 */
    const char msg[] = "OSAL_BOOT_FATAL kind=hard-fault\r\n";
    const char *p = msg;
    while (*p) {
        /* Spin while UART TX full.                                   */
        while ((*(volatile uint32_t *)0x40004004U) & 1U) { }
        *(volatile uint32_t *)(0x40004000U) = (uint32_t)(unsigned char)*p;
        p++;
    }
    qemu_exit_failure();
}

/* ------------------------------------------------------------------ */
/* Runtime image initialisation                                       */
/* ------------------------------------------------------------------ */
static void runtime_image_init(void)
{
    const uint32_t *source      = &_sidata;
    uint32_t       *destination = &_sdata;

    /* Copy .data from FLASH to RAM.                                  */
    while (destination < &_edata) {
        *destination++ = *source++;
    }

    /* Zero .bss.                                                      */
    destination = &_sbss;
    while (destination < &_ebss) {
        *destination++ = 0U;
    }

    /* Data synchronisation barrier for the ARM memory model.         */
    __asm volatile ("dsb");
    __asm volatile ("isb");
}

/* ------------------------------------------------------------------ */
/* Reset_Handler — entry point.                                       */
/*                                                                     */
/* The initial stack pointer is set by the vector table; a normal      */
/* C function prologue / epilogue is safe here.  noreturn because      */
/* main() must never return.                                           */
/* ------------------------------------------------------------------ */
__attribute__((noreturn))
void Reset_Handler(void)
{
    /* 1. Initialise the runtime image (.data copy, .bss zero).       */
    runtime_image_init();

    /* 2. Call main().  If it returns, treat as fatal.                */
    {
        int result = main();

        const char msg[] = "OSAL_BOOT_FATAL kind=main-returned\r\n";
        const char *p = msg;
        while (*p) {
            while ((*(volatile uint32_t *)0x40004004U) & 1U) { }
            *(volatile uint32_t *)(0x40004000U) = (uint32_t)(unsigned char)*p;
            p++;
        }
        (void)result;
    }
    qemu_exit_failure();

    for (;;) {
        __asm volatile ("wfi");
    }
}

/* ------------------------------------------------------------------ */
/* Vector table — generated by the linker via .isr_vector section.   */
/* ------------------------------------------------------------------ */
__attribute__((section(".isr_vector")))
void (* const _vectors[])(void) = {
    (void (*)(void))((uint32_t)0x20000000U + 4U * 1024U * 1024U), /* SP */
    Reset_Handler,              /* -15 Reset                        */
    (void *)0,                  /* -14 NMI                          */
    HardFault_Handler,          /* -13 HardFault                    */
    Default_Handler,            /* -12 MemManage                    */
    Default_Handler,            /* -11 BusFault                     */
    Default_Handler,            /* -10 UsageFault                   */
    (void *)0,                  /*  -9 reserved                     */
    (void *)0,                  /*  -8 reserved                     */
    (void *)0,                  /*  -7 reserved                     */
    (void *)0,                  /*  -6 reserved                     */
    vPortSVCHandler,            /*  -5 SVC — FreeRTOS uses this      */
    Default_Handler,            /*  -4 DebugMon                     */
    (void *)0,                  /*  -3 reserved                     */
    xPortPendSVHandler,         /*  -2 PendSV                       */
    xPortSysTickHandler,        /*  -1 SysTick                      */
    /* IRQ 0–15                                                        */
    TIMER0_Handler,             /*   0 TIMER0                        */
    TIMER1_Handler,             /*   1 TIMER1                        */
    Default_Handler,            /*   2 TIMER2                        */
    Default_Handler,            /*   3 TIMER3                        */
    Default_Handler,            /*   4 UART0 combined                 */
    Default_Handler,            /*   5 UART1 combined                 */
    Default_Handler,            /*   6 UART2 combined                 */
    Default_Handler,            /*   7 UART3 combined                 */
    Default_Handler,            /*   8 UART4 combined                 */
    Default_Handler,            /*   9 reserved                     */
    Default_Handler,            /*  10 reserved                     */
    Default_Handler,            /*  11 reserved                     */
    Default_Handler,            /*  12 reserved                     */
    Default_Handler,            /*  13 reserved                     */
    Default_Handler,            /*  14 reserved                     */
    Default_Handler,            /*  15 reserved                     */
};
