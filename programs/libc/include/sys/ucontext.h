#ifndef _SYS_UCONTEXT_H
#define _SYS_UCONTEXT_H

/* Minimal stub for TCC compilation */
typedef struct {
    unsigned int eip;
    unsigned int esp;
    unsigned int ebp;
} mcontext_t;

typedef struct ucontext {
    mcontext_t uc_mcontext;
} ucontext_t;

#endif
