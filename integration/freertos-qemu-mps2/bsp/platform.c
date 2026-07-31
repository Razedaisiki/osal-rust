/* platform.c — MPS2-AN385 platform initialisation stub.
 *
 * The vendor startup (startup_gcc.c) already copies .data, zeroes .bss,
 * and calls SystemInit() which configures the system clock.  This file
 * exists so main() has a single initialisation call before the console
 * and scheduler are set up.
 */

#include "platform.h"

void platform_init(void)
{
    /* SystemInit() has already been called by startup_gcc.c before
     * main().  No additional platform setup is needed for C-only boot. */
}
