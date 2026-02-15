/* Linker script for libfont.dll — loaded at 0x04200000 */
ENTRY(_dll_start)

SECTIONS {
    . = 0x04200000;

    .rodata : {
        KEEP(*(.exports))
        *(.rodata .rodata.*)
    }

    .text : {
        *(.text._dll_start)
        *(.text .text.*)
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note*)
        *(.data .data.* .bss .bss.*)
    }
}
