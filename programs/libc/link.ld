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
