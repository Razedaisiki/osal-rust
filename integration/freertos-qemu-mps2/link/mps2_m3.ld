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
 *   - None.  Rust output sections (.ARM.exidx, .ARM.extab, etc.) will
 *     be added in a future step when the Rust staticlib is integrated.
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
        *(.text)
        *(.rodata*)
        *(.constdata*)
        _etext = .;
        _sidata = .;
    } > FLASH

    .data :
    {
        . = ALIGN(8);
        _data = .;
        _sdata = .;
        *(vtable)
        *(.data)
        _edata = .;
    } > RAM

    .bss :
    {
        . = ALIGN(8);
        _bss = .;
        _sbss = .;
        *(.bss)
        _ebss = .;
    } > RAM

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
    } >RAM

    __StackTop = ORIGIN(RAM) + LENGTH(RAM);
    __StackLimit = __StackTop - _Min_Stack_Size;
    PROVIDE(__stack = __StackTop);

    ASSERT(__StackLimit >= _heap_top, "region RAM overflowed with stack")
}
