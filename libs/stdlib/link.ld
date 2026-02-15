/* Linker script for .anyOS user programs (flat binary at 0x08000000) */
ENTRY(_start)

SECTIONS {
    . = 0x08000000;

    .text : {
        /* Ensure _start is at the very beginning */
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
        *(.bss .bss.* COMMON)
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note*)
    }
}
