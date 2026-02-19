ENTRY(_boot_start)
OUTPUT_FORMAT(elf64-x86-64)

KERNEL_VMA = 0xFFFFFFFF80100000;
KERNEL_LMA = 0x00100000;

SECTIONS
{
    . = KERNEL_VMA;

    .text ALIGN(4K) : AT(ADDR(.text) - KERNEL_VMA + KERNEL_LMA)
    {
        _kernel_start = .;
        *(.text.boot)
        *(.text .text.*)
    }

    .rodata ALIGN(4K) : AT(ADDR(.rodata) - KERNEL_VMA + KERNEL_LMA)
    {
        *(.rodata .rodata.*)
    }

    .data ALIGN(4K) : AT(ADDR(.data) - KERNEL_VMA + KERNEL_LMA)
    {
        *(.data .data.*)
    }

    .bss ALIGN(4K) : AT(ADDR(.bss) - KERNEL_VMA + KERNEL_LMA)
    {
        _bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        _bss_end = .;
    }

    /* Boot stack: 512 KiB dedicated region placed above BSS.
     * Separating the stack from BSS ensures a stack overflow cannot silently
     * corrupt static data (GDT, IDT, IST stacks, etc.).
     * _boot_stack_top is the initial RSP; the stack grows downward from there.
     * _boot_stack_bottom serves as a sentinel: if RSP ever drops below it,
     * the stack has overflowed into BSS. */
    .boot_stack (NOLOAD) : AT(ADDR(.boot_stack) - KERNEL_VMA + KERNEL_LMA)
    {
        _boot_stack_bottom = .;
        . += 0x80000;    /* 512 KiB */
        _boot_stack_top = .;
    }

    _kernel_end = .;

    /DISCARD/ :
    {
        *(.comment)
        *(.eh_frame)
        *(.note.*)
    }
}
