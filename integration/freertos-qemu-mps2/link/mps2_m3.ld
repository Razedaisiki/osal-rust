/*
 * Linker script for MPS2-AN385 Cortex-M3 (QEMU).
 *
 * Original source:
 *   Repository: https://github.com/FreeRTOS/FreeRTOS
 *   Commit:     592732b4d8e8da21f122322de4c421f89e0b4d18
 *   Path:       FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/build/gcc/mps2_m3.ld
 *   License:    MIT
 *
 * Modifications from original:
 *   - .text* / .rodata* wildcards for Rust staticlib sections
 *   - .data* / .rel.ro* wildcards with FLASH LMA and RAM VMA
 *   - .bss* / COMMON wildcards with NOLOAD
 *   - .ARM.extab / .ARM.exidx for Rust unwind tables
 *   - Linker assertions for section bounds
 *
 * Original FreeRTOS copyright:
 *   FreeRTOS V202212.00
 *   Copyright (C) 2020 Amazon.com, Inc. or its affiliates. All Rights Reserved.
 *   Licensed under the MIT License.
 */

MEMORY
{
    FLASH (xr) : ORIGIN = 0x00000000, LENGTH = 4M
    RAM (rw)  : ORIGIN = 0x20000000, LENGTH = 4M
}
ENTRY(Reset_Handler)

_Min_Heap_Size = 0x8;
_Min_Stack_Size = 0x400;
_estack = ORIGIN(RAM) + LENGTH(RAM);


SECTIONS
{
    .isr_vector :
    {
        __vector_table = .;
        KEEP(*(.isr_vector))
        . = ALIGN(4);
    } > FLASH

    .text :
    {
        . = ALIGN(4);
        *(.text)
        *(.text*)
        *(.rodata)
        *(.rodata*)
        *(.constdata*)
        . = ALIGN(4);
    } > FLASH

    _etext = .;

    /* .data: runtime address in RAM, load address in FLASH.           */
    .data :
    {
        . = ALIGN(8);
        _sdata = .;

        *(.data)
        *(.data*)
        *(.data.rel.ro*)
        *(vtable)

        . = ALIGN(8);
        _edata = .;
    } > RAM AT > FLASH

    _sidata = LOADADDR(.data);

    /* .bss: zero-initialised at runtime by Reset_Handler.             */
    .bss (NOLOAD) :
    {
        . = ALIGN(8);
        _sbss = .;

        *(.bss)
        *(.bss*)
        *(COMMON)

        . = ALIGN(8);
        _ebss = .;
    } > RAM

    /* Rust exception unwind tables.                                   */
    .ARM.extab :
    {
        *(.ARM.extab*)
    } > FLASH

    .ARM.exidx :
    {
        __exidx_start = .;
        *(.ARM.exidx*)
        __exidx_end = .;
    } > FLASH

    .heap :
    {
        . = ALIGN(8);
        PROVIDE ( end = . );
        PROVIDE ( _end = . );
        _heap_bottom = .;
        . = . + _Min_Heap_Size;
        _heap_top = .;
        . = . + _Min_Stack_Size;
        . = ALIGN(8);
    } > RAM

    __StackTop = ORIGIN(RAM) + LENGTH(RAM);
    __StackLimit = __StackTop - _Min_Stack_Size;
    PROVIDE(__stack = __StackTop);

    /* ------------------------------------------------------------------ */
    /* Assertions                                                         */
    /* ------------------------------------------------------------------ */
    ASSERT(_sidata >= ORIGIN(FLASH), ".data load address is not in FLASH")
    ASSERT(_sdata >= ORIGIN(RAM),   ".data runtime address is not in RAM")
    ASSERT(_edata <= ORIGIN(RAM) + LENGTH(RAM), ".data exceeds RAM")
    ASSERT(_sbss >= ORIGIN(RAM),    ".bss runtime address is not in RAM")
    ASSERT(_ebss <= ORIGIN(RAM) + LENGTH(RAM), ".bss exceeds RAM")
    ASSERT(__StackLimit >= _heap_top, "region RAM overflowed with stack")
}
