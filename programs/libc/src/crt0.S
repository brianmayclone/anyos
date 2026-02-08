.section .text
.global _start
_start:
    /* Clear frame pointer for backtraces */
    xorl %ebp, %ebp
    /* Call main(argc=0, argv=NULL) */
    pushl $0     /* argv */
    pushl $0     /* argc */
    call main
    addl $8, %esp
    /* Call exit(return_value) */
    pushl %eax
    call exit
    /* Should not reach here */
    hlt
