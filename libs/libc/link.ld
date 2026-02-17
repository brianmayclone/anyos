/* Linker script for anyOS C programs (ELF at 0x08000000) */
ENTRY(_start)

SECTIONS {
    . = 0x08000000;

    .text : {
        *(.text._start)
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.*)
    }

    .init_array : {
        __init_array_start = .;
        KEEP(*(.init_array .init_array.* .ctors))
        __init_array_end = .;
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
        *(.eh_frame)
        *(.comment)
        *(.note*)
    }
}
