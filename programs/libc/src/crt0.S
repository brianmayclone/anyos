.section .text
.global _start
_start:
    /* Clear frame pointer for backtraces */
    xorl %ebp, %ebp
    /* __libc_start_main parses args and calls main(argc, argv) */
    call __libc_start_main
    /* Should not reach here (__libc_start_main calls exit) */
    hlt
