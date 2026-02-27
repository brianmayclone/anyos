/* libc64 linker script — 64-bit user programs for anyOS */
/* Base address 0x08000000 matches kernel's ELF loader expectations. */

ENTRY(_start)

SECTIONS
{
    . = 0x08000000;

    .text : {
        *(.text._start)
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.*)
    }

    /* C++ exception unwinding tables */
    .eh_frame : {
        __eh_frame_start = .;
        *(.eh_frame .eh_frame.*)
        __eh_frame_end = .;
    }

    .eh_frame_hdr : {
        *(.eh_frame_hdr .eh_frame_hdr.*)
    }

    .gcc_except_table : {
        *(.gcc_except_table .gcc_except_table.*)
    }

    /* C/C++ constructor/destructor arrays */
    .init_array : {
        __init_array_start = .;
        KEEP(*(.init_array))
        KEEP(*(SORT(.init_array.*)))
        __init_array_end = .;
    }

    .fini_array : {
        __fini_array_start = .;
        KEEP(*(.fini_array))
        KEEP(*(SORT(.fini_array.*)))
        __fini_array_end = .;
    }

    .data : {
        *(.data .data.*)
    }

    .bss : {
        __bss_start = .;
        *(.bss .bss.* COMMON)
        __bss_end = .;
    }

    _end = .;

    /DISCARD/ : {
        *(.comment)
        *(.note*)
    }
}
