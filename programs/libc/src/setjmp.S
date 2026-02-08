.section .text

/* int setjmp(jmp_buf env) */
.global setjmp
setjmp:
    movl 4(%esp), %ecx      /* ecx = env pointer */
    movl %ebx, 0(%ecx)      /* save ebx */
    movl %esi, 4(%ecx)      /* save esi */
    movl %edi, 8(%ecx)      /* save edi */
    movl %ebp, 12(%ecx)     /* save ebp */
    leal 4(%esp), %eax      /* compute saved esp (before call pushed ret addr) */
    movl %eax, 16(%ecx)     /* save esp */
    movl (%esp), %eax       /* return address */
    movl %eax, 20(%ecx)     /* save eip */
    xorl %eax, %eax         /* return 0 */
    ret

/* void longjmp(jmp_buf env, int val) */
.global longjmp
longjmp:
    movl 4(%esp), %ecx      /* ecx = env pointer */
    movl 8(%esp), %eax      /* eax = val */
    testl %eax, %eax
    jnz 1f
    incl %eax               /* if val == 0, return 1 */
1:
    movl 0(%ecx), %ebx      /* restore ebx */
    movl 4(%ecx), %esi      /* restore esi */
    movl 8(%ecx), %edi      /* restore edi */
    movl 12(%ecx), %ebp     /* restore ebp */
    movl 16(%ecx), %esp     /* restore esp */
    jmp *20(%ecx)            /* jump to saved eip */
