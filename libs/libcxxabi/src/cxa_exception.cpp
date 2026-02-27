/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * cxa_exception.cpp -- Core C++ exception handling for anyOS.
 *
 * Implements the Itanium C++ ABI exception allocation, throw, catch,
 * and the __gxx_personality_v0 personality routine that reads LSDA
 * (Language-Specific Data Area) tables emitted by clang.
 *
 * This file is compiled with -fno-exceptions because it IS the
 * exception mechanism -- it must never throw itself.
 *
 * Key data flow:
 *   throw expr  ->  __cxa_allocate_exception + construct + __cxa_throw
 *   __cxa_throw ->  _Unwind_RaiseException  (phase 1: search, phase 2: cleanup)
 *   personality ->  reads LSDA, matches catch types, installs landing pads
 *   catch (T& e) -> __cxa_begin_catch ... __cxa_end_catch
 *
 * Reference: https://itanium-cxx-abi.github.io/cxx-abi/abi-eh.html
 */

#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

#include <unwind.h>
#include <cxxabi.h>

/* ────────────────────────────────────────────────────────────────────── */
/*  Exception class identifier                                           */
/* ────────────────────────────────────────────────────────────────────── */

/*
 * The Itanium ABI uses an 8-byte exception_class field so that the
 * personality routine can distinguish "our" C++ exceptions from foreign
 * ones (e.g. SEH, Objective-C).  Clang / libcxxabi uses "CLNGC++\0".
 */
static constexpr uint64_t kOurExceptionClass =
    (static_cast<uint64_t>('C') << 56) |
    (static_cast<uint64_t>('L') << 48) |
    (static_cast<uint64_t>('N') << 40) |
    (static_cast<uint64_t>('G') << 32) |
    (static_cast<uint64_t>('C') << 24) |
    (static_cast<uint64_t>('+') << 16) |
    (static_cast<uint64_t>('+') <<  8) |
    (static_cast<uint64_t>('\0'));

/* ────────────────────────────────────────────────────────────────────── */
/*  __cxa_exception -- ABI header placed BEFORE the thrown object         */
/* ────────────────────────────────────────────────────────────────────── */

/*
 * Memory layout:
 *   [ __cxa_exception header ] [ user's thrown object ... ]
 *                               ^
 *                               pointer returned by __cxa_allocate_exception
 *
 * The _Unwind_Exception (unwindHeader) is at the END of __cxa_exception
 * so that (&unwindHeader + 1) == (user object).  This is the Itanium ABI
 * convention and allows the personality routine to recover the
 * __cxa_exception from the _Unwind_Exception* that the unwinder passes.
 */

struct __cxa_exception {
    std::type_info       *exceptionType;
    void                (*exceptionDestructor)(void *);
    void                (*unexpectedHandler)();
    void                (*terminateHandler)();
    __cxa_exception      *nextException;
    int                   handlerCount;
    int                   handlerSwitchValue;
    const char           *actionRecord;
    const char           *languageSpecificData;
    void                 *catchTemp;
    void                 *adjustedPtr;
    _Unwind_Exception     unwindHeader;
};

/* ────────────────────────────────────────────────────────────────────── */
/*  Per-thread exception globals                                         */
/* ────────────────────────────────────────────────────────────────────── */

/*
 * In a full OS with real TLS we would use __thread or thread_local.
 * anyOS currently has a single-threaded C++ user-space model, so a
 * plain static is sufficient.  When TLS lands this should become
 * thread_local.
 */

struct __cxa_eh_globals {
    __cxa_exception *caughtExceptions;    /* stack of caught exceptions */
    unsigned int     uncaughtExceptions;  /* count of in-flight exceptions */
};

static __cxa_eh_globals eh_globals = { nullptr, 0 };

/**
 * Return a pointer to the per-thread EH globals.
 */
static __cxa_eh_globals *get_globals() {
    return &eh_globals;
}

/* ────────────────────────────────────────────────────────────────────── */
/*  Helpers: __cxa_exception <-> user pointer <-> _Unwind_Exception      */
/* ────────────────────────────────────────────────────────────────────── */

/**
 * Given the pointer to the user's thrown object, return the ABI header.
 */
static inline __cxa_exception *exception_from_thrown(void *thrown) {
    return static_cast<__cxa_exception *>(thrown) - 1;
}

/**
 * Given a pointer to the unwindHeader inside __cxa_exception, recover
 * the enclosing __cxa_exception.
 */
static inline __cxa_exception *exception_from_unwind(
        _Unwind_Exception *unwind_exception) {
    /* unwindHeader is the last field of __cxa_exception. */
    return reinterpret_cast<__cxa_exception *>(
        reinterpret_cast<char *>(unwind_exception) -
        offsetof(__cxa_exception, unwindHeader));
}

/**
 * Given the ABI header, return the user's thrown object pointer.
 */
static inline void *thrown_from_exception(__cxa_exception *exc) {
    return static_cast<void *>(exc + 1);
}

/* ────────────────────────────────────────────────────────────────────── */
/*  Exception allocation                                                 */
/* ────────────────────────────────────────────────────────────────────── */

extern "C" void *__cxa_allocate_exception(size_t thrown_size) {
    /*
     * Total allocation = header + thrown object.
     * The header must be aligned to at least the alignment of
     * _Unwind_Exception (8 or 16 bytes depending on platform).
     */
    size_t total = sizeof(__cxa_exception) + thrown_size;
    void *raw = malloc(total);
    if (!raw) {
        /* Out of memory during throw -- unrecoverable. */
        fprintf(stderr, "libcxxabi: failed to allocate exception (%zu bytes)\n",
                total);
        abort();
    }

    /* Zero-initialise the entire block (header + object). */
    memset(raw, 0, total);

    /* Return pointer to the user object (past the header). */
    __cxa_exception *header = static_cast<__cxa_exception *>(raw);
    return thrown_from_exception(header);
}

extern "C" void __cxa_free_exception(void *thrown_exception) {
    if (!thrown_exception) return;
    __cxa_exception *header = exception_from_thrown(thrown_exception);
    free(static_cast<void *>(header));
}

/* ────────────────────────────────────────────────────────────────────── */
/*  __cxa_throw                                                          */
/* ────────────────────────────────────────────────────────────────────── */

/* std::terminate is defined in cxa_handlers.cpp */
namespace std { [[noreturn]] void terminate() noexcept; }

/**
 * Cleanup callback invoked by the unwinder if the exception is foreign-caught.
 */
static void exception_cleanup(_Unwind_Reason_Code /*reason*/,
                              _Unwind_Exception *unwind_exception) {
    __cxa_exception *exc = exception_from_unwind(unwind_exception);
    void *thrown = thrown_from_exception(exc);
    if (exc->exceptionDestructor) {
        exc->exceptionDestructor(thrown);
    }
    __cxa_free_exception(thrown);
}

extern "C" void __cxa_throw(void *thrown_exception,
                             std::type_info *tinfo,
                             void (*dest)(void *)) {
    __cxa_exception *header = exception_from_thrown(thrown_exception);

    header->exceptionType       = tinfo;
    header->exceptionDestructor = dest;
    header->unexpectedHandler   = nullptr;
    header->terminateHandler    = nullptr;

    /* Initialise the unwind header. */
    header->unwindHeader.exception_class   = kOurExceptionClass;
    header->unwindHeader.exception_cleanup = exception_cleanup;

    __cxa_eh_globals *globals = get_globals();
    globals->uncaughtExceptions++;

    /*
     * Start two-phase unwinding.  _Unwind_RaiseException returns only
     * if no handler was found (should not happen for C++ exceptions
     * because std::terminate is implicitly installed).
     */
    _Unwind_Reason_Code rc =
        _Unwind_RaiseException(&header->unwindHeader);

    /* If we get here, unwinding failed entirely. */
    (void)rc;
    fprintf(stderr, "libcxxabi: _Unwind_RaiseException failed (rc=%d), "
            "calling std::terminate()\n", static_cast<int>(rc));
    std::terminate();
}

/* ────────────────────────────────────────────────────────────────────── */
/*  __cxa_begin_catch / __cxa_end_catch                                  */
/* ────────────────────────────────────────────────────────────────────── */

extern "C" void *__cxa_begin_catch(void *exception_object) {
    _Unwind_Exception *unwind_exception =
        static_cast<_Unwind_Exception *>(exception_object);

    /*
     * Check whether this is one of our C++ exceptions or a foreign one.
     */
    bool is_native = (unwind_exception->exception_class == kOurExceptionClass);

    if (is_native) {
        __cxa_exception *exc = exception_from_unwind(unwind_exception);
        __cxa_eh_globals *globals = get_globals();

        /* Increment handler count. */
        exc->handlerCount++;

        /*
         * If this is the first catch for this exception, push it onto
         * the caught-exceptions stack and decrement uncaught count.
         */
        if (exc->handlerCount == 1) {
            exc->nextException = globals->caughtExceptions;
            globals->caughtExceptions = exc;

            if (globals->uncaughtExceptions > 0) {
                globals->uncaughtExceptions--;
            }
        }

        return exc->adjustedPtr;
    }

    /*
     * Foreign exception -- we cannot do much, but the ABI says we must
     * push *something*.  We store nullptr as the caught exception and
     * return the unwind header pointer as the "caught object".
     */
    __cxa_eh_globals *globals = get_globals();
    if (globals->uncaughtExceptions > 0) {
        globals->uncaughtExceptions--;
    }
    return exception_object;
}

extern "C" void __cxa_end_catch() {
    __cxa_eh_globals *globals = get_globals();
    __cxa_exception *exc = globals->caughtExceptions;
    if (!exc) return;

    exc->handlerCount--;

    if (exc->handlerCount == 0) {
        /* Pop from the caught stack. */
        globals->caughtExceptions = exc->nextException;
        exc->nextException = nullptr;

        /* Destroy the thrown object and free. */
        void *thrown = thrown_from_exception(exc);
        if (exc->exceptionDestructor) {
            exc->exceptionDestructor(thrown);
        }
        __cxa_free_exception(thrown);
    }
}

/* ────────────────────────────────────────────────────────────────────── */
/*  __cxa_rethrow                                                        */
/* ────────────────────────────────────────────────────────────────────── */

extern "C" void __cxa_rethrow() {
    __cxa_eh_globals *globals = get_globals();
    __cxa_exception *exc = globals->caughtExceptions;
    if (!exc) {
        fprintf(stderr, "libcxxabi: __cxa_rethrow called with no current "
                "exception\n");
        std::terminate();
    }

    /*
     * Mark as re-thrown: increment uncaught count (it was decremented
     * by __cxa_begin_catch) and decrement handler count (the catch
     * block will not reach __cxa_end_catch because we are re-throwing).
     */
    globals->uncaughtExceptions++;
    exc->handlerCount--;

    /* Pop from caught stack if handler count dropped to zero. */
    if (exc->handlerCount == 0) {
        globals->caughtExceptions = exc->nextException;
        exc->nextException = nullptr;
    }

    _Unwind_Resume(&exc->unwindHeader);

    /* _Unwind_Resume should never return. */
    fprintf(stderr, "libcxxabi: _Unwind_Resume returned in __cxa_rethrow\n");
    std::terminate();
}

/* ────────────────────────────────────────────────────────────────────── */
/*  __cxa_get_exception_ptr / __cxa_current_primary_exception            */
/* ────────────────────────────────────────────────────────────────────── */

extern "C" void *__cxa_get_exception_ptr(void *exception_object) {
    _Unwind_Exception *unwind_exception =
        static_cast<_Unwind_Exception *>(exception_object);
    if (unwind_exception->exception_class == kOurExceptionClass) {
        __cxa_exception *exc = exception_from_unwind(unwind_exception);
        return exc->adjustedPtr;
    }
    return exception_object;
}

extern "C" void *__cxa_current_primary_exception() {
    __cxa_eh_globals *globals = get_globals();
    __cxa_exception *exc = globals->caughtExceptions;
    if (!exc) return nullptr;

    exc->handlerCount++;
    return &exc->unwindHeader;
}

/* ════════════════════════════════════════════════════════════════════════
 *  DWARF / LSDA parsing helpers
 *
 *  The LSDA (Language-Specific Data Area) is a binary blob in the
 *  .gcc_except_table section, pointed to by the FDE augmentation data.
 *  Its format is defined by the Itanium C++ ABI and uses DWARF pointer
 *  encodings extensively.
 * ════════════════════════════════════════════════════════════════════════ */

/* ── DWARF pointer encoding constants ────────────────────────────────── */

enum {
    DW_EH_PE_absptr  = 0x00,
    DW_EH_PE_uleb128 = 0x01,
    DW_EH_PE_udata2  = 0x02,
    DW_EH_PE_udata4  = 0x03,
    DW_EH_PE_udata8  = 0x04,
    DW_EH_PE_sleb128 = 0x09,
    DW_EH_PE_sdata2  = 0x0A,
    DW_EH_PE_sdata4  = 0x0B,
    DW_EH_PE_sdata8  = 0x0C,

    DW_EH_PE_pcrel   = 0x10,
    DW_EH_PE_textrel = 0x20,
    DW_EH_PE_datarel = 0x30,
    DW_EH_PE_funcrel = 0x40,
    DW_EH_PE_aligned = 0x50,

    DW_EH_PE_indirect = 0x80,
    DW_EH_PE_omit     = 0xFF
};

/* ── ULEB128 / SLEB128 decoding ──────────────────────────────────────── */

/**
 * Decode an unsigned LEB128 value from *data, advancing the pointer.
 */
static uint64_t read_uleb128(const uint8_t **data) {
    uint64_t result = 0;
    unsigned shift = 0;
    uint8_t byte;
    do {
        byte = **data;
        (*data)++;
        result |= static_cast<uint64_t>(byte & 0x7F) << shift;
        shift += 7;
    } while (byte & 0x80);
    return result;
}

/**
 * Decode a signed LEB128 value from *data, advancing the pointer.
 */
static int64_t read_sleb128(const uint8_t **data) {
    int64_t result = 0;
    unsigned shift = 0;
    uint8_t byte;
    do {
        byte = **data;
        (*data)++;
        result |= static_cast<int64_t>(byte & 0x7F) << shift;
        shift += 7;
    } while (byte & 0x80);

    /* Sign-extend if the high bit of the last byte was set. */
    if ((shift < 64) && (byte & 0x40)) {
        result |= -(static_cast<int64_t>(1) << shift);
    }
    return result;
}

/* ── Encoded pointer reading ─────────────────────────────────────────── */

/**
 * Read a DWARF-encoded pointer from *data, advancing the pointer.
 *
 * @param encoding  The DW_EH_PE_* encoding byte.
 * @param data      Pointer to the current read position (updated).
 * @param base      Base address for pcrel calculations (address of the
 *                  encoded value itself in memory).
 * @return The decoded pointer value, or 0 if encoding is DW_EH_PE_omit.
 */
static uintptr_t read_encoded_pointer(const uint8_t **data,
                                       uint8_t encoding) {
    if (encoding == DW_EH_PE_omit) return 0;

    const uint8_t *start = *data;
    uintptr_t result = 0;

    /* ── Step 1: read the raw value based on the low nibble ─────────── */
    switch (encoding & 0x0F) {
        case DW_EH_PE_absptr:
            result = *reinterpret_cast<const uintptr_t *>(*data);
            *data += sizeof(uintptr_t);
            break;
        case DW_EH_PE_uleb128:
            result = static_cast<uintptr_t>(read_uleb128(data));
            break;
        case DW_EH_PE_sleb128:
            result = static_cast<uintptr_t>(read_sleb128(data));
            break;
        case DW_EH_PE_udata2:
            result = *reinterpret_cast<const uint16_t *>(*data);
            *data += 2;
            break;
        case DW_EH_PE_udata4:
            result = *reinterpret_cast<const uint32_t *>(*data);
            *data += 4;
            break;
        case DW_EH_PE_udata8:
            result = *reinterpret_cast<const uint64_t *>(*data);
            *data += 8;
            break;
        case DW_EH_PE_sdata2:
            result = static_cast<uintptr_t>(
                *reinterpret_cast<const int16_t *>(*data));
            *data += 2;
            break;
        case DW_EH_PE_sdata4:
            result = static_cast<uintptr_t>(
                *reinterpret_cast<const int32_t *>(*data));
            *data += 4;
            break;
        case DW_EH_PE_sdata8:
            result = static_cast<uintptr_t>(
                *reinterpret_cast<const int64_t *>(*data));
            *data += 8;
            break;
        default:
            /* Unknown value encoding -- treat as absptr and hope. */
            result = *reinterpret_cast<const uintptr_t *>(*data);
            *data += sizeof(uintptr_t);
            break;
    }

    /* If the raw value is zero, it means "no value" regardless of rel. */
    if (result == 0) return 0;

    /* ── Step 2: apply the relocation based on the high nibble ──────── */
    switch (encoding & 0x70) {
        case 0:  /* DW_EH_PE_absptr -- absolute, no relocation */
            break;
        case DW_EH_PE_pcrel:
            result += reinterpret_cast<uintptr_t>(start);
            break;
        case DW_EH_PE_textrel:
            /* Requires text-segment base; not used by clang on x86_64. */
            break;
        case DW_EH_PE_datarel:
            /* Requires data-segment base; not used by clang on x86_64. */
            break;
        case DW_EH_PE_funcrel:
            /* Function-relative; personality adds func start itself. */
            break;
        case DW_EH_PE_aligned: {
            /* Align *data to pointer-size boundary first. */
            uintptr_t addr = reinterpret_cast<uintptr_t>(*data);
            addr = (addr + sizeof(uintptr_t) - 1) & ~(sizeof(uintptr_t) - 1);
            *data = reinterpret_cast<const uint8_t *>(addr);
            result = *reinterpret_cast<const uintptr_t *>(*data);
            *data += sizeof(uintptr_t);
            break;
        }
        default:
            break;
    }

    /* ── Step 3: indirect -- dereference the pointer ────────────────── */
    if (encoding & DW_EH_PE_indirect) {
        result = *reinterpret_cast<const uintptr_t *>(result);
    }

    return result;
}

/* ════════════════════════════════════════════════════════════════════════
 *  LSDA structure
 *
 *  The LSDA has this layout:
 *
 *  [header]
 *    uint8_t   lp_start_encoding    (encoding for landing-pad base)
 *    encoded   lp_start             (if encoding != DW_EH_PE_omit)
 *    uint8_t   tt_encoding          (encoding for type table entries)
 *    uleb128   tt_offset            (if encoding != DW_EH_PE_omit;
 *                                    byte offset from HERE to type table)
 *    uint8_t   cs_encoding          (encoding for call-site entries)
 *    uleb128   cs_table_length      (byte length of call-site table)
 *
 *  [call-site table]
 *    For each call site:
 *      encoded  cs_start            (offset from lp_start to region start)
 *      encoded  cs_len              (length of the region)
 *      encoded  cs_lp               (offset from lp_start to landing pad;
 *                                    0 = no landing pad)
 *      uleb128  cs_action           (1-based index into action table;
 *                                    0 = cleanup only)
 *
 *  [action table]
 *    Each record is:
 *      sleb128  type_filter          (+N = catch, 0 = cleanup, -N = filter)
 *      sleb128  next_action_offset   (byte offset to next record, 0 = end)
 *
 *  [type table]
 *    Array of encoded pointers to std::type_info, indexed from the END
 *    of the table (filter index N reads entry at offset -N).
 * ════════════════════════════════════════════════════════════════════════ */

/**
 * Parsed LSDA header -- computed once per personality call.
 */
struct lsda_header_t {
    uintptr_t       lp_start;          /* landing pad base address */
    const uint8_t  *type_table;        /* pointer to end of type table */
    uint8_t         tt_encoding;       /* encoding of type table entries */
    const uint8_t  *call_site_table;   /* start of call site table */
    const uint8_t  *call_site_end;     /* end of call site table */
    uint8_t         cs_encoding;       /* encoding of call site entries */
    const uint8_t  *action_table;      /* start of action table */
};

/**
 * Parse the LSDA header starting at lsda_ptr.
 */
static void parse_lsda_header(const uint8_t *lsda_ptr,
                                uintptr_t func_start,
                                lsda_header_t *out) {
    const uint8_t *p = lsda_ptr;

    /* Landing-pad start encoding. */
    uint8_t lp_start_encoding = *p++;
    if (lp_start_encoding != DW_EH_PE_omit) {
        out->lp_start = read_encoded_pointer(&p, lp_start_encoding);
    } else {
        /* Default: landing pads are relative to function start. */
        out->lp_start = func_start;
    }

    /* Type table encoding and offset. */
    out->tt_encoding = *p++;
    if (out->tt_encoding != DW_EH_PE_omit) {
        uint64_t tt_offset = read_uleb128(&p);
        out->type_table = p + tt_offset;
    } else {
        out->type_table = nullptr;
    }

    /* Call-site table encoding and length. */
    out->cs_encoding = *p++;
    uint64_t cs_length = read_uleb128(&p);
    out->call_site_table = p;
    out->call_site_end   = p + cs_length;
    out->action_table    = p + cs_length;
}

/**
 * Return the size in bytes of a single encoded value with the given
 * encoding's value part (low nibble).  For LEB128 this is unknown
 * a priori so we return 0 (caller must use read_encoded_pointer).
 */
static size_t encoded_value_size(uint8_t encoding) {
    switch (encoding & 0x0F) {
        case DW_EH_PE_absptr:  return sizeof(uintptr_t);
        case DW_EH_PE_udata2:
        case DW_EH_PE_sdata2:  return 2;
        case DW_EH_PE_udata4:
        case DW_EH_PE_sdata4:  return 4;
        case DW_EH_PE_udata8:
        case DW_EH_PE_sdata8:  return 8;
        default:               return 0; /* LEB128 or unknown */
    }
}

/**
 * Look up a type_info pointer in the type table.
 *
 * @param type_table   Pointer to the END of the type table.
 * @param tt_encoding  Encoding of each entry.
 * @param filter_index Positive 1-based index (filter > 0).
 * @return Pointer to the std::type_info, or nullptr for catch-all.
 */
static const std::type_info *get_type_info(const uint8_t *type_table,
                                            uint8_t tt_encoding,
                                            int64_t filter_index) {
    if (!type_table || filter_index <= 0) return nullptr;

    /*
     * The type table is an array of encoded pointers stored BEFORE
     * type_table.  Entry with filter index 1 is at (type_table - 1*size),
     * index 2 at (type_table - 2*size), etc.
     */
    size_t entry_size = encoded_value_size(tt_encoding);
    if (entry_size == 0) {
        /* LEB128-encoded type table entries (uncommon but possible). */
        entry_size = sizeof(uintptr_t);
    }

    const uint8_t *entry = type_table - filter_index * entry_size;
    uintptr_t ptr = read_encoded_pointer(&entry, tt_encoding);
    return reinterpret_cast<const std::type_info *>(ptr);
}

/**
 * Check whether the thrown exception's type matches the catch type.
 *
 * For now we use a simple name-based comparison, which handles:
 *   - Exact type matches
 *   - catch (...) represented by nullptr catch_type
 *
 * A full implementation would walk the RTTI inheritance graph, but
 * name comparison covers the vast majority of real-world use cases.
 */
static bool exception_type_matches(const std::type_info *throw_type,
                                    const std::type_info *catch_type,
                                    void *thrown_ptr,
                                    void **adjusted_ptr) {
    /* catch (...) matches everything. */
    if (!catch_type) {
        *adjusted_ptr = thrown_ptr;
        return true;
    }

    /* No type info on the thrown exception -- cannot match. */
    if (!throw_type) {
        return false;
    }

    /*
     * Compare mangled names.  The Itanium ABI guarantees that identical
     * types share the same mangled name string, and on most bare-metal
     * targets the linker coalesces them to the same address.  We check
     * address first, then fall back to strcmp.
     */
    if (throw_type == catch_type) {
        *adjusted_ptr = thrown_ptr;
        return true;
    }
    if (throw_type->name() && catch_type->name() &&
        strcmp(throw_type->name(), catch_type->name()) == 0) {
        *adjusted_ptr = thrown_ptr;
        return true;
    }

    /*
     * Base class matching: check if catch_type is a public base of
     * throw_type by walking the __si_class_type_info chain.
     */
    const auto *si = dynamic_cast<const __cxxabiv1::__si_class_type_info *>(
        static_cast<const __cxxabiv1::__class_type_info *>(throw_type));
    while (si) {
        const std::type_info *base = si->__base_type;
        if (base == catch_type) {
            *adjusted_ptr = thrown_ptr;
            return true;
        }
        if (base->name() && catch_type->name() &&
            strcmp(base->name(), catch_type->name()) == 0) {
            *adjusted_ptr = thrown_ptr;
            return true;
        }
        /* Walk up the single-inheritance chain. */
        si = dynamic_cast<const __cxxabiv1::__si_class_type_info *>(
            static_cast<const __cxxabiv1::__class_type_info *>(base));
    }

    return false;
}

/* ════════════════════════════════════════════════════════════════════════
 *  __gxx_personality_v0 -- the C++ personality routine
 *
 *  Called by the unwinder for every frame that has LSDA data.
 *
 *  Phase 1 (_UA_SEARCH_PHASE):
 *    Scan call sites to find one matching the current IP.  If found and
 *    it has an action that matches the thrown type, return
 *    _URC_HANDLER_FOUND.  Otherwise _URC_CONTINUE_UNWIND.
 *
 *  Phase 2 (_UA_CLEANUP_PHASE):
 *    Same scan, but now we install the landing pad.  If this is the
 *    handler frame (_UA_HANDLER_FRAME), set the switch value so the
 *    landing pad knows which catch clause to enter.  For cleanup-only
 *    landing pads, set switch value to 0.
 *
 *  Register conventions for x86_64:
 *    GR[0] = RAX = exception object pointer
 *    GR[1] = RDX = switch value (selector)
 * ════════════════════════════════════════════════════════════════════════ */

/* x86_64 DWARF register numbers for the two ABI-defined GP registers. */
static constexpr int UNWIND_REG_EXCEPTION_PTR = 0;  /* RAX */
static constexpr int UNWIND_REG_SWITCH_VALUE  = 1;  /* RDX */

extern "C" _Unwind_Reason_Code __gxx_personality_v0(
        int version,
        _Unwind_Action actions,
        uint64_t exception_class,
        _Unwind_Exception *unwind_exception,
        _Unwind_Context *context) {

    /* We only understand version 1 of the personality protocol. */
    if (version != 1) return _URC_FATAL_PHASE1_ERROR;

    /* Get the LSDA for this frame. */
    const uint8_t *lsda_ptr = reinterpret_cast<const uint8_t *>(
        _Unwind_GetLanguageSpecificData(context));
    if (!lsda_ptr) return _URC_CONTINUE_UNWIND;

    uintptr_t func_start = _Unwind_GetRegionStart(context);

    /*
     * The unwinder gives us the return address, which points to the
     * instruction AFTER the call.  Subtract 1 to get an address
     * within the call instruction itself, so the call-site range check
     * [cs_start, cs_start + cs_len) is correct.
     */
    uintptr_t ip = _Unwind_GetIP(context) - 1;
    uintptr_t ip_offset = ip - func_start;

    /* Parse the LSDA header. */
    lsda_header_t lsda;
    parse_lsda_header(lsda_ptr, func_start, &lsda);

    /* Determine if this is one of our C++ exceptions. */
    bool is_native = (exception_class == kOurExceptionClass);

    /* Get the __cxa_exception header for native exceptions. */
    __cxa_exception *cxa_exc = nullptr;
    const std::type_info *throw_type = nullptr;
    void *thrown_ptr = nullptr;

    if (is_native && unwind_exception) {
        cxa_exc = exception_from_unwind(unwind_exception);
        throw_type = cxa_exc->exceptionType;
        thrown_ptr = thrown_from_exception(cxa_exc);
    }

    /* ── Scan call-site table ────────────────────────────────────────── */
    const uint8_t *cs = lsda.call_site_table;

    while (cs < lsda.call_site_end) {
        /* Read one call-site entry. */
        uintptr_t cs_start  = read_encoded_pointer(&cs, lsda.cs_encoding);
        uintptr_t cs_len    = read_encoded_pointer(&cs, lsda.cs_encoding);
        uintptr_t cs_lp     = read_encoded_pointer(&cs, lsda.cs_encoding);
        uint64_t  cs_action = read_uleb128(&cs);

        /* Does this call site cover the current IP? */
        if (ip_offset < cs_start || ip_offset >= cs_start + cs_len) {
            continue;
        }

        /* No landing pad -> no handler or cleanup for this site. */
        if (cs_lp == 0) {
            return _URC_CONTINUE_UNWIND;
        }

        uintptr_t landing_pad = lsda.lp_start + cs_lp;

        /*
         * cs_action == 0 means cleanup only (no catch).
         * cs_action >  0 means (cs_action - 1) is the byte offset into
         *                the action table.
         */
        if (cs_action == 0) {
            /* Cleanup landing pad -- only relevant in phase 2. */
            if (actions & _UA_SEARCH_PHASE) {
                return _URC_CONTINUE_UNWIND;
            }
            /* Phase 2: install the cleanup landing pad. */
            _Unwind_SetGR(context, UNWIND_REG_EXCEPTION_PTR,
                          reinterpret_cast<uint64_t>(unwind_exception));
            _Unwind_SetGR(context, UNWIND_REG_SWITCH_VALUE, 0);
            _Unwind_SetIP(context, landing_pad);
            return _URC_INSTALL_CONTEXT;
        }

        /* ── Walk the action table ───────────────────────────────────── */
        const uint8_t *action_entry =
            lsda.action_table + (cs_action - 1);

        while (true) {
            const uint8_t *action_pos = action_entry;

            int64_t type_filter = read_sleb128(&action_entry);
            int64_t next_offset = read_sleb128(&action_entry);

            if (type_filter > 0) {
                /*
                 * Positive filter -> catch clause.
                 * Look up the type_info in the type table.
                 */
                const std::type_info *catch_type =
                    get_type_info(lsda.type_table, lsda.tt_encoding,
                                 type_filter);

                void *adjusted = nullptr;
                bool matches = false;

                if (!catch_type) {
                    /* nullptr type_info means catch (...) */
                    matches = true;
                    adjusted = thrown_ptr;
                } else if (is_native) {
                    matches = exception_type_matches(
                        throw_type, catch_type, thrown_ptr, &adjusted);
                }

                if (matches) {
                    if (actions & _UA_SEARCH_PHASE) {
                        /* Phase 1: we found a handler. */
                        if (cxa_exc) {
                            cxa_exc->handlerSwitchValue =
                                static_cast<int>(type_filter);
                            cxa_exc->actionRecord =
                                reinterpret_cast<const char *>(action_pos);
                            cxa_exc->languageSpecificData =
                                reinterpret_cast<const char *>(lsda_ptr);
                            cxa_exc->catchTemp =
                                reinterpret_cast<void *>(landing_pad);
                            cxa_exc->adjustedPtr = adjusted;
                        }
                        return _URC_HANDLER_FOUND;
                    }

                    /* Phase 2 + HANDLER_FRAME: install the handler. */
                    if (actions & _UA_HANDLER_FRAME) {
                        if (cxa_exc) {
                            cxa_exc->adjustedPtr = adjusted;
                        }
                        _Unwind_SetGR(context, UNWIND_REG_EXCEPTION_PTR,
                                      reinterpret_cast<uint64_t>(
                                          unwind_exception));
                        _Unwind_SetGR(context, UNWIND_REG_SWITCH_VALUE,
                                      static_cast<uint64_t>(type_filter));
                        _Unwind_SetIP(context, landing_pad);
                        return _URC_INSTALL_CONTEXT;
                    }
                }
            } else if (type_filter == 0) {
                /*
                 * Filter value 0 = cleanup action.
                 * Only install in phase 2 (not search phase).
                 */
                if (actions & _UA_CLEANUP_PHASE) {
                    _Unwind_SetGR(context, UNWIND_REG_EXCEPTION_PTR,
                                  reinterpret_cast<uint64_t>(
                                      unwind_exception));
                    _Unwind_SetGR(context, UNWIND_REG_SWITCH_VALUE, 0);
                    _Unwind_SetIP(context, landing_pad);
                    return _URC_INSTALL_CONTEXT;
                }
            } else {
                /*
                 * Negative filter = exception specification filter.
                 * For C++17 onwards these are mostly obsolete
                 * (noexcept is enforced differently).  We treat a
                 * mismatch as a call to std::terminate via the
                 * cleanup path.
                 */
                if (actions & _UA_CLEANUP_PHASE) {
                    _Unwind_SetGR(context, UNWIND_REG_EXCEPTION_PTR,
                                  reinterpret_cast<uint64_t>(
                                      unwind_exception));
                    _Unwind_SetGR(context, UNWIND_REG_SWITCH_VALUE,
                                  static_cast<uint64_t>(type_filter));
                    _Unwind_SetIP(context, landing_pad);
                    return _URC_INSTALL_CONTEXT;
                }
            }

            /* Move to next action record, or stop if there are no more. */
            if (next_offset == 0) break;
            action_entry = action_pos + next_offset;
        }

        /*
         * We found the call site but no action matched.  If we are in
         * the cleanup phase and there is a landing pad, install it as
         * a cleanup.
         */
        if (actions & _UA_CLEANUP_PHASE) {
            _Unwind_SetGR(context, UNWIND_REG_EXCEPTION_PTR,
                          reinterpret_cast<uint64_t>(unwind_exception));
            _Unwind_SetGR(context, UNWIND_REG_SWITCH_VALUE, 0);
            _Unwind_SetIP(context, landing_pad);
            return _URC_INSTALL_CONTEXT;
        }

        return _URC_CONTINUE_UNWIND;
    }

    /* No call site matched the current IP. */
    return _URC_CONTINUE_UNWIND;
}
