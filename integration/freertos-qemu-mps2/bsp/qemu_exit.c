/* qemu_exit.c — ARM semihosting QEMU exit.
 *
 * Uses ARM semihosting SYS_EXIT (0x18) via the SVC 0xAB
 * instruction.  QEMU interprets:
 *   r0 = operation (0x18 = SYS_EXIT)
 *   r1 = exit type:
 *        ADP_Stopped_ApplicationExit (0x20026) → exit(0)
 *        anything else                        → exit(1)
 */

#include "qemu_exit.h"

#define SYS_EXIT                      0x18
#define ADP_STOPPED_APPLICATION_EXIT  0x20026

static void semihosting_exit(int code)
{
    int param;

    if (code == 0) {
        param = (int)ADP_STOPPED_APPLICATION_EXIT;  /* → exit(0) */
    } else {
        param = 1;                                  /* → exit(1) */
    }

    __asm__ volatile (
        "mov  r0, #0x18\n"
        "mov  r1, %[p]\n"
        "svc  0xAB\n"
        :
        : [p] "r" (param)
        : "r0", "r1", "memory"
    );

    /* Should never reach here. */
    for (;;) {
        __asm__ volatile ("wfi");
    }
}

void qemu_exit_success(void)
{
    semihosting_exit(0);
}

void qemu_exit_failure(void)
{
    semihosting_exit(1);
}
