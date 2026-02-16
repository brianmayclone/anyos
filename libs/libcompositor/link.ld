/* Linker script for libcompositor.dlib — loaded at 0x04380000 */
ENTRY(_dll_start)

SECTIONS {
    . = 0x04380000;

    /* Shared read-only region (same physical pages across all processes) */
    .rodata : {
        KEEP(*(.exports))
        *(.rodata .rodata.*)
    }

    .text : {
        *(.text._dll_start)
        *(.text .text.*)
    }

    /* Page-align before per-process writable sections */
    . = ALIGN(4096);

    /* Per-process .data (each process gets its own copy from template) */
    .data : {
        *(.data .data.*)
    }

    . = ALIGN(4096);

    /* Per-process .bss (zeroed on demand per process) */
    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    }

    . = ALIGN(4096);

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.note*)
    }
}
