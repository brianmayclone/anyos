#ifndef _SETJMP_H
#define _SETJMP_H

/* i686: save ebx, esi, edi, ebp, esp, eip = 6 regs = 24 bytes */
typedef unsigned int jmp_buf[6];

int setjmp(jmp_buf env);
void longjmp(jmp_buf env, int val) __attribute__((noreturn));

#endif
