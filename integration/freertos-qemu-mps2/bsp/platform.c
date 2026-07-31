/* platform.c — MPS2-AN385 platform initialisation.
 *
 * The frozen MPS2 QEMU startup transfers control directly to main().
 * No additional platform initialisation is required for the current
 * C-only QEMU ELF boot.
 *
 * When the Rust staticlib is integrated, the .data / .bss / stack
 * initialisation and .ARM.exidx semantics must be explicitly confirmed.
 */

#include "platform.h"

void platform_init(void)
{
    /* No platform-level setup required for C-only ELF boot on QEMU. */
}
