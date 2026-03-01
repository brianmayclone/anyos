/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * <cxxabi.h> -- Itanium C++ ABI runtime interface for anyOS.
 *
 * Declares the functions that the compiler-generated exception handling
 * code and RTTI machinery call at runtime.  This is the public contract
 * between clang++ codegen and the C++ ABI support library (libcxxabi).
 *
 * Reference: Itanium C++ ABI, rev 1.86
 *            https://itanium-cxx-abi.github.io/cxx-abi/abi.html
 */

#ifndef __CXXABI_H
#define __CXXABI_H

#include <stddef.h>
#include <stdint.h>
#include <typeinfo>

/* Guard variable type: at least 64 bits, first byte is the state byte. */
typedef uint64_t __cxa_guard_type;

#ifdef __cplusplus
extern "C" {
#endif

/* ── Exception allocation / throw / catch ────────────────────────────── */

/**
 * Allocate memory for an exception object of the given size.
 * The returned pointer points to the user's thrown object; the ABI
 * header (__cxa_exception) is placed immediately before it.
 */
void *__cxa_allocate_exception(size_t thrown_size);

/**
 * Free an exception previously allocated with __cxa_allocate_exception.
 */
void __cxa_free_exception(void *thrown_exception);

/**
 * Begin exception propagation.
 *
 * @param thrown_exception  Pointer returned by __cxa_allocate_exception
 *                          (the user object, already constructed).
 * @param tinfo             Pointer to the std::type_info of the thrown type.
 * @param dest              Destructor for the thrown object, or nullptr.
 *
 * Fills in the __cxa_exception header, then calls _Unwind_RaiseException.
 * If the unwinder fails to find a handler, calls std::terminate().
 */
void __cxa_throw(void *thrown_exception, std::type_info *tinfo,
                 void (*dest)(void *));

/**
 * Enter a catch clause.
 *
 * Called by compiler-generated landing-pad code at the start of a catch
 * block.  Pushes the exception onto the per-thread caught-exceptions
 * stack and returns the adjusted pointer to the caught object.
 */
void *__cxa_begin_catch(void *exception_object);

/**
 * Leave a catch clause.
 *
 * Decrements the handler count; when it reaches zero the exception is
 * destroyed (destructor called) and freed.
 */
void __cxa_end_catch(void);

/**
 * Re-throw the currently caught exception.
 * Must be called from inside a catch block (between begin/end_catch).
 */
void __cxa_rethrow(void);

/**
 * Get the adjusted pointer for the exception without entering a catch.
 * Used by catch-block type matching during phase 1 (search).
 */
void *__cxa_get_exception_ptr(void *exception_object);

/**
 * Return the current primary (innermost) exception, incrementing its
 * reference count so it can be stored as an exception_ptr.
 */
void *__cxa_current_primary_exception(void);

/* ── Static local guard variables ────────────────────────────────────── */

/**
 * Acquire the initialisation lock for a static local variable.
 *
 * @return 1 if the caller must perform the initialisation, 0 if another
 *         thread (or previous call) already completed it.
 */
int __cxa_guard_acquire(__cxa_guard_type *guard);

/**
 * Release the guard after successful initialisation.
 */
void __cxa_guard_release(__cxa_guard_type *guard);

/**
 * Abort the guard after a failed initialisation (exception thrown
 * from the initialiser).
 */
void __cxa_guard_abort(__cxa_guard_type *guard);

/* ── atexit / finalize ───────────────────────────────────────────────── */

/**
 * Register a destructor to be called at program exit (or DSO unload).
 *
 * @param func  Destructor function.
 * @param arg   Argument passed to func.
 * @param dso   DSO handle (unused on anyOS, pass nullptr).
 * @return 0 on success, -1 if the table is full.
 */
int __cxa_atexit(void (*func)(void *), void *arg, void *dso);

/**
 * Run all destructors registered for the given DSO handle (or all
 * destructors if dso is nullptr).
 */
void __cxa_finalize(void *dso);

/* ── Pure / deleted virtual call handlers ────────────────────────────── */

/**
 * Called when a pure virtual function is invoked (should never happen
 * in correct programs).  Prints an error and aborts.
 */
void __cxa_pure_virtual(void);

/**
 * Called when a deleted virtual function is invoked.
 * Prints an error and aborts.
 */
void __cxa_deleted_virtual(void);

/* ── Personality routine ──────────────────────────────────────────────
 * __gxx_personality_v0 is declared/defined in cxa_exception.cpp which
 * includes <unwind.h> for the full _Unwind_Reason_Code type.
 * ──────────────────────────────────────────────────────────────────── */

#ifdef __cplusplus
} /* extern "C" */
#endif

/* ── RTTI class hierarchy (namespace __cxxabiv1) ─────────────────────── */

namespace __cxxabiv1 {

/**
 * Base type_info for class types with no bases.
 */
class __class_type_info : public std::type_info {
public:
    explicit __class_type_info(const char *name);
    ~__class_type_info() override;
};

/**
 * Type info for classes with a single, public, non-virtual base.
 */
class __si_class_type_info : public __class_type_info {
public:
    const __class_type_info *__base_type;

    explicit __si_class_type_info(const char *name,
                                  const __class_type_info *base);
    ~__si_class_type_info() override;
};

/**
 * Base-class descriptor used by __vmi_class_type_info.
 */
struct __base_class_type_info {
    const __class_type_info *__base_type;
    long __offset_flags;

    enum __offset_flags_masks {
        __virtual_mask  = 0x1,
        __public_mask   = 0x2,
        __offset_shift  = 8
    };
};

/**
 * Type info for classes with virtual or multiple inheritance.
 */
class __vmi_class_type_info : public __class_type_info {
public:
    unsigned int __flags;
    unsigned int __base_count;
    __base_class_type_info __base_info[1]; /* variable-length */

    enum __flags_masks {
        __non_diamond_repeat_mask = 0x1,
        __diamond_shaped_mask     = 0x2
    };

    explicit __vmi_class_type_info(const char *name, unsigned int flags,
                                   unsigned int base_count);
    ~__vmi_class_type_info() override;
};

/**
 * Type info for fundamental types (int, float, etc.).
 */
class __fundamental_type_info : public std::type_info {
public:
    explicit __fundamental_type_info(const char *name);
    ~__fundamental_type_info() override;
};

/**
 * Type info for pointer types.
 */
class __pointer_type_info : public std::type_info {
public:
    unsigned int __flags;
    const std::type_info *__pointee;

    enum __masks {
        __const_mask            = 0x01,
        __volatile_mask         = 0x02,
        __restrict_mask         = 0x04,
        __incomplete_mask       = 0x08,
        __incomplete_class_mask = 0x10
    };

    explicit __pointer_type_info(const char *name, unsigned int flags,
                                 const std::type_info *pointee);
    ~__pointer_type_info() override;
};

/**
 * Demangle a mangled C++ symbol name.
 */
extern "C" char *__cxa_demangle(const char *mangled_name, char *output_buffer,
                                size_t *length, int *status);

} /* namespace __cxxabiv1 */

namespace abi = __cxxabiv1;

#endif /* __CXXABI_H */
