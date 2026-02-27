/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * unwind.h — Itanium C++ ABI Unwind Interface for x86_64 anyOS.
 *
 * Defines the types and function prototypes required by the Itanium C++ ABI
 * exception handling specification (Level I: Base ABI). This header is
 * consumed by the C++ personality routine (__gxx_personality_v0) and by
 * compiler-generated landing-pad code.
 *
 * Reference: https://itanium-cxx-abi.github.io/cxx-abi/abi-eh.html
 */

#ifndef _UNWIND_H
#define _UNWIND_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ------------------------------------------------------------------ */
/*  Reason codes returned by _Unwind_* functions and personality       */
/* ------------------------------------------------------------------ */

typedef enum {
    _URC_NO_REASON                = 0,
    _URC_FOREIGN_EXCEPTION_CAUGHT = 1,
    _URC_FATAL_PHASE2_ERROR       = 2,
    _URC_FATAL_PHASE1_ERROR       = 3,
    _URC_NORMAL_STOP              = 4,
    _URC_END_OF_STACK             = 5,
    _URC_HANDLER_FOUND            = 6,
    _URC_INSTALL_CONTEXT          = 7,
    _URC_CONTINUE_UNWIND          = 8
} _Unwind_Reason_Code;

/* ------------------------------------------------------------------ */
/*  Action flags passed to personality routines during each phase       */
/* ------------------------------------------------------------------ */

typedef int _Unwind_Action;

#define _UA_SEARCH_PHASE  1
#define _UA_CLEANUP_PHASE 2
#define _UA_HANDLER_FRAME 4
#define _UA_FORCE_UNWIND  8

/* ------------------------------------------------------------------ */
/*  Exception object — allocated by the language runtime (e.g. libcxxabi) */
/* ------------------------------------------------------------------ */

struct _Unwind_Exception;

/** Cleanup function invoked when a foreign exception is caught. */
typedef void (*_Unwind_Exception_Cleanup_Fn)(
    _Unwind_Reason_Code reason,
    struct _Unwind_Exception *exc);

/**
 * Portable exception header embedded at the start of every thrown object.
 * The struct must be naturally aligned to 8 bytes (or more) so that the
 * language runtime can place it at any malloc'd address.
 */
struct _Unwind_Exception {
    uint64_t                       exception_class;
    _Unwind_Exception_Cleanup_Fn   exception_cleanup;
    uint64_t                       private_1;
    uint64_t                       private_2;
} __attribute__((aligned(8)));

/* ------------------------------------------------------------------ */
/*  Opaque cursor / context — represents a single stack frame          */
/* ------------------------------------------------------------------ */

struct _Unwind_Context;

/* ------------------------------------------------------------------ */
/*  Personality routine typedef                                        */
/* ------------------------------------------------------------------ */

/**
 * Each function that contains landing pads points (via its CIE/FDE
 * augmentation) to a personality routine.  The unwinder calls it once
 * per frame during each phase.
 */
typedef _Unwind_Reason_Code (*_Unwind_Personality_Fn)(
    int                          version,
    _Unwind_Action               actions,
    uint64_t                     exception_class,
    struct _Unwind_Exception    *exception_object,
    struct _Unwind_Context      *context);

/* ------------------------------------------------------------------ */
/*  Core unwind entry points                                           */
/* ------------------------------------------------------------------ */

/**
 * Begin two-phase exception unwinding.
 *
 * Phase 1 (search): walk frames calling personality with _UA_SEARCH_PHASE
 *   until one returns _URC_HANDLER_FOUND.
 * Phase 2 (cleanup): walk frames again calling personality with
 *   _UA_CLEANUP_PHASE; the handler frame additionally receives
 *   _UA_HANDLER_FRAME.
 *
 * Returns _URC_END_OF_STACK if no handler is found.
 */
_Unwind_Reason_Code _Unwind_RaiseException(struct _Unwind_Exception *exception_object);

/**
 * Resume propagation after a cleanup (non-catching) landing pad.
 * Called by compiler-generated code at the end of a cleanup; does not return.
 */
void _Unwind_Resume(struct _Unwind_Exception *exception_object)
    __attribute__((noreturn));

/**
 * Release resources associated with an exception object.
 * Calls the exception_cleanup callback if non-NULL.
 */
void _Unwind_DeleteException(struct _Unwind_Exception *exception_object);

/* ------------------------------------------------------------------ */
/*  Context accessors — used by personality routines                    */
/* ------------------------------------------------------------------ */

/** Get a general-purpose register value (DWARF register number). */
uint64_t _Unwind_GetGR(struct _Unwind_Context *context, int reg_index);

/** Set a general-purpose register value (DWARF register number). */
void _Unwind_SetGR(struct _Unwind_Context *context, int reg_index, uint64_t value);

/** Get the instruction pointer (return address) for this frame. */
uint64_t _Unwind_GetIP(struct _Unwind_Context *context);

/** Set the instruction pointer — used to redirect into a landing pad. */
void _Unwind_SetIP(struct _Unwind_Context *context, uint64_t new_ip);

/** Return a pointer to the language-specific data area (LSDA) for this frame. */
uint64_t _Unwind_GetLanguageSpecificData(struct _Unwind_Context *context);

/** Return the start address of the procedure (function) containing this frame. */
uint64_t _Unwind_GetRegionStart(struct _Unwind_Context *context);

/** Return the canonical frame address (CFA) for this frame. */
uint64_t _Unwind_GetCFA(struct _Unwind_Context *context);

/* ------------------------------------------------------------------ */
/*  Backtrace callback interface                                       */
/* ------------------------------------------------------------------ */

/**
 * Callback type for _Unwind_Backtrace.
 * Return _URC_NO_REASON to continue, anything else to stop.
 */
typedef _Unwind_Reason_Code (*_Unwind_Trace_Fn)(
    struct _Unwind_Context *context,
    void                   *arg);

/**
 * Walk the call stack invoking the callback for each frame.
 * Useful for debug/diagnostic backtraces.
 */
_Unwind_Reason_Code _Unwind_Backtrace(_Unwind_Trace_Fn callback, void *arg);

#ifdef __cplusplus
}
#endif

#endif /* _UNWIND_H */
