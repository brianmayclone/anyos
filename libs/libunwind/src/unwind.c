/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * unwind.c — Minimal DWARF .eh_frame stack unwinder for x86_64 anyOS.
 *
 * Implements the Itanium C++ ABI Level I (Base ABI) unwinding interface.
 * Parses .eh_frame CIE/FDE records, executes DWARF Call Frame Instructions
 * (CFI) to compute caller register state, and performs two-phase exception
 * unwinding (search for handler, then cleanup + transfer to landing pad).
 *
 * Limitations (by design — keeps the implementation small):
 *   - x86_64 only (DWARF register numbers 0-16)
 *   - Linear FDE scan (no .eh_frame_hdr binary search)
 *   - Only CFI opcodes emitted by clang for x86_64 are supported
 *   - Single-threaded state stack for DW_CFA_remember/restore_state
 *   - No forced unwinding (_UA_FORCE_UNWIND is accepted but not initiated)
 */

#include <stdint.h>
#include <stddef.h>
#include <string.h>

#include "unwind.h"

/* ================================================================== */
/*  Forward declarations for assembly helpers (unwind_registers.S)     */
/* ================================================================== */

/**
 * Defined in unwind_registers.S.
 * The public _Unwind_RaiseException is an ASM trampoline that saves the
 * caller's registers into an unwind_cursor_t, then calls this C function.
 */
_Unwind_Reason_Code _Unwind_RaiseException_impl(
    struct _Unwind_Exception *exception_object,
    void *cursor_ptr);

/**
 * Restore callee-saved registers from the cursor and jump to cursor->rip.
 * Does not return.  Defined in unwind_registers.S.
 */
void _unwind_restore_and_jump(void *cursor_ptr) __attribute__((noreturn));

/* ================================================================== */
/*  .eh_frame linker symbols                                           */
/* ================================================================== */

extern char __eh_frame_start[];
extern char __eh_frame_end[];

/* ================================================================== */
/*  x86_64 DWARF register numbers                                     */
/* ================================================================== */

#define DWARF_RAX  0
#define DWARF_RDX  1
#define DWARF_RCX  2
#define DWARF_RBX  3
#define DWARF_RSI  4
#define DWARF_RDI  5
#define DWARF_RBP  6
#define DWARF_RSP  7
#define DWARF_R8   8
#define DWARF_R9   9
#define DWARF_R10 10
#define DWARF_R11 11
#define DWARF_R12 12
#define DWARF_R13 13
#define DWARF_R14 14
#define DWARF_R15 15
#define DWARF_RA  16  /* Return address — mapped to RIP */

#define DWARF_REG_COUNT 17

/* ================================================================== */
/*  Unwind cursor — represents one stack frame's register state        */
/* ================================================================== */

/**
 * Internal representation of a stack frame's saved register state.
 * This struct is also used as the opaque _Unwind_Context passed to
 * personality routines and context accessors.
 */
typedef struct {
    uint64_t regs[DWARF_REG_COUNT];  /* Indexed by DWARF register number */

    /* Metadata from FDE/CIE for the current frame */
    uint64_t func_start;   /* Initial location from FDE (function start) */
    uint64_t lsda;         /* Language-Specific Data Area pointer */
    _Unwind_Personality_Fn personality;  /* Personality routine pointer */
} unwind_cursor_t;

/* ================================================================== */
/*  CFA (Canonical Frame Address) rule                                 */
/* ================================================================== */

/** How the CFA is computed for a given PC location. */
typedef struct {
    int      reg;     /* DWARF register number used as base */
    int64_t  offset;  /* Signed offset added to register value */
} cfa_rule_t;

/* ================================================================== */
/*  Register save rules (result of executing CFI instructions)         */
/* ================================================================== */

/** Rule type for a single register's save location. */
typedef enum {
    RULE_UNDEFINED,    /* Register value is undefined / not saved */
    RULE_SAME_VALUE,   /* Register retains its current value */
    RULE_OFFSET,       /* Saved at CFA + offset (signed) */
    RULE_REGISTER,     /* Value is in another register */
    RULE_VAL_OFFSET    /* Value IS CFA + offset (not dereferenced) */
} reg_rule_type_t;

typedef struct {
    reg_rule_type_t type;
    int64_t         value;  /* Offset or register number depending on type */
} reg_rule_t;

/** Complete register save state at a given PC within a function. */
typedef struct {
    cfa_rule_t  cfa;
    reg_rule_t  rules[DWARF_REG_COUNT];
} reg_state_t;

/* ================================================================== */
/*  State stack for DW_CFA_remember_state / DW_CFA_restore_state       */
/* ================================================================== */

#define STATE_STACK_DEPTH 8

static reg_state_t state_stack[STATE_STACK_DEPTH];
static int         state_stack_top = 0;

/* ================================================================== */
/*  Parsed CIE (Common Information Entry)                              */
/* ================================================================== */

typedef struct {
    uint8_t        version;
    uint64_t       code_align;        /* Code alignment factor (ULEB128) */
    int64_t        data_align;        /* Data alignment factor (SLEB128) */
    uint64_t       ra_reg;            /* Return address register */
    uint8_t        fde_encoding;      /* FDE pointer encoding (DW_EH_PE_*) */
    uint8_t        lsda_encoding;     /* LSDA pointer encoding */
    uint8_t        has_augmentation;  /* Parsed 'z' augmentation? */
    _Unwind_Personality_Fn personality;
    const uint8_t *initial_instructions;
    uint64_t       initial_instructions_len;
} parsed_cie_t;

/* ================================================================== */
/*  Parsed FDE (Frame Description Entry)                               */
/* ================================================================== */

typedef struct {
    const uint8_t  *cie_ptr;          /* Pointer to the owning CIE */
    uint64_t        pc_begin;         /* Start address of described range */
    uint64_t        pc_range;         /* Length of described address range */
    uint64_t        lsda;             /* LSDA pointer (0 if absent) */
    const uint8_t  *instructions;     /* CFI instruction stream */
    uint64_t        instructions_len;
    parsed_cie_t    cie;              /* Copy of the parsed CIE */
} parsed_fde_t;

/* ================================================================== */
/*  DWARF pointer encodings (DW_EH_PE_*)                               */
/* ================================================================== */

#define DW_EH_PE_absptr   0x00
#define DW_EH_PE_uleb128  0x01
#define DW_EH_PE_udata2   0x02
#define DW_EH_PE_udata4   0x03
#define DW_EH_PE_udata8   0x04
#define DW_EH_PE_sleb128  0x09
#define DW_EH_PE_sdata2   0x0A
#define DW_EH_PE_sdata4   0x0B
#define DW_EH_PE_sdata8   0x0C

#define DW_EH_PE_pcrel    0x10
#define DW_EH_PE_datarel  0x30

#define DW_EH_PE_indirect 0x80
#define DW_EH_PE_omit     0xFF

/* ================================================================== */
/*  DWARF CFA instruction opcodes                                      */
/* ================================================================== */

/* High 2 bits encode the primary opcode, low 6 bits are the operand */
#define DW_CFA_advance_loc_hi      0x40  /* delta in low 6 bits */
#define DW_CFA_offset_hi           0x80  /* register in low 6 bits */
#define DW_CFA_restore_hi          0xC0  /* register in low 6 bits */

/* Extended opcodes (high 2 bits == 0) */
#define DW_CFA_nop                 0x00
#define DW_CFA_set_loc             0x01
#define DW_CFA_advance_loc1        0x02
#define DW_CFA_advance_loc2        0x03
#define DW_CFA_advance_loc4        0x04
#define DW_CFA_offset_extended     0x05
#define DW_CFA_restore_extended    0x06
#define DW_CFA_undefined           0x07
#define DW_CFA_same_value          0x08
#define DW_CFA_register            0x09
#define DW_CFA_remember_state      0x0A
#define DW_CFA_restore_state       0x0B
#define DW_CFA_def_cfa             0x0C
#define DW_CFA_def_cfa_register    0x0D
#define DW_CFA_def_cfa_offset      0x0E
#define DW_CFA_def_cfa_expression  0x0F
#define DW_CFA_expression          0x10
#define DW_CFA_offset_extended_sf  0x11
#define DW_CFA_def_cfa_sf          0x12
#define DW_CFA_def_cfa_offset_sf   0x13
#define DW_CFA_val_offset          0x14
#define DW_CFA_val_offset_sf       0x15
#define DW_CFA_val_expression      0x16
#define DW_CFA_GNU_args_size       0x2E
#define DW_CFA_GNU_negative_offset_extended 0x2F

/* ================================================================== */
/*  LEB128 decoders                                                    */
/* ================================================================== */

/**
 * Decode an unsigned LEB128 value.
 * Advances *p past the consumed bytes.
 */
static uint64_t decode_uleb128(const uint8_t **p)
{
    uint64_t result = 0;
    unsigned shift = 0;

    for (;;) {
        uint8_t byte = **p;
        (*p)++;
        result |= (uint64_t)(byte & 0x7F) << shift;
        if ((byte & 0x80) == 0)
            break;
        shift += 7;
        if (shift >= 64)
            break;  /* Malformed — prevent infinite loop */
    }

    return result;
}

/**
 * Decode a signed LEB128 value.
 * Advances *p past the consumed bytes.
 */
static int64_t decode_sleb128(const uint8_t **p)
{
    int64_t  result = 0;
    unsigned shift  = 0;
    uint8_t  byte;

    for (;;) {
        byte = **p;
        (*p)++;
        result |= (int64_t)(byte & 0x7F) << shift;
        shift += 7;
        if ((byte & 0x80) == 0)
            break;
        if (shift >= 64)
            break;  /* Malformed — prevent infinite loop */
    }

    /* Sign-extend if the highest bit of the last byte was set */
    if ((shift < 64) && (byte & 0x40))
        result |= -(1LL << shift);

    return result;
}

/* ================================================================== */
/*  Encoded pointer reader                                             */
/* ================================================================== */

/**
 * Read a pointer value encoded with DW_EH_PE_* encoding.
 * @param p        Current read position (advanced past the value)
 * @param encoding DW_EH_PE_* encoding byte
 * @param base     PC-relative base address (address of the encoded value itself)
 * @return         Decoded pointer value, or 0 on DW_EH_PE_omit
 */
static uint64_t read_encoded_pointer(const uint8_t **p, uint8_t encoding,
                                     uint64_t base)
{
    if (encoding == DW_EH_PE_omit)
        return 0;

    const uint8_t *start = *p;
    uint64_t result;

    /* Decode the value portion (low 4 bits of encoding) */
    switch (encoding & 0x0F) {
    case DW_EH_PE_absptr:
        memcpy(&result, *p, 8);
        *p += 8;
        break;
    case DW_EH_PE_uleb128:
        result = decode_uleb128(p);
        break;
    case DW_EH_PE_sleb128:
        result = (uint64_t)decode_sleb128(p);
        break;
    case DW_EH_PE_udata2: {
        uint16_t v;
        memcpy(&v, *p, 2);
        *p += 2;
        result = v;
        break;
    }
    case DW_EH_PE_udata4: {
        uint32_t v;
        memcpy(&v, *p, 4);
        *p += 4;
        result = v;
        break;
    }
    case DW_EH_PE_udata8: {
        uint64_t v;
        memcpy(&v, *p, 8);
        *p += 8;
        result = v;
        break;
    }
    case DW_EH_PE_sdata2: {
        int16_t v;
        memcpy(&v, *p, 2);
        *p += 2;
        result = (uint64_t)(int64_t)v;
        break;
    }
    case DW_EH_PE_sdata4: {
        int32_t v;
        memcpy(&v, *p, 4);
        *p += 4;
        result = (uint64_t)(int64_t)v;
        break;
    }
    case DW_EH_PE_sdata8: {
        int64_t v;
        memcpy(&v, *p, 8);
        *p += 8;
        result = (uint64_t)v;
        break;
    }
    default:
        return 0;  /* Unsupported value encoding */
    }

    /* Apply the relative modifier (bits 4-6) */
    switch (encoding & 0x70) {
    case 0:  /* DW_EH_PE_absptr — no adjustment */
        break;
    case DW_EH_PE_pcrel:
        result += (uint64_t)start;
        break;
    case DW_EH_PE_datarel:
        result += base;
        break;
    default:
        break;  /* Unsupported application encoding */
    }

    /* Indirect: result is a pointer to the actual value */
    if (encoding & DW_EH_PE_indirect) {
        uint64_t deref;
        memcpy(&deref, (const void *)result, 8);
        result = deref;
    }

    return result;
}

/* ================================================================== */
/*  CIE parser                                                         */
/* ================================================================== */

/**
 * Parse a CIE (Common Information Entry) from .eh_frame.
 *
 * CIE layout (after length+id fields which the caller has consumed):
 *   version              (1 byte)
 *   augmentation string  (NUL-terminated)
 *   code_alignment       (ULEB128)
 *   data_alignment       (SLEB128)
 *   return_addr_register (ULEB128 for version >= 3, 1 byte for version 1)
 *   [augmentation data]  (if augmentation starts with 'z')
 *   initial_instructions (rest of CIE)
 *
 * @param data      Pointer to the CIE data (after length + CIE_id)
 * @param cie_len   Remaining bytes in the CIE record
 * @param out       Output parsed CIE structure
 * @return          0 on success, -1 on parse error
 */
static int parse_cie(const uint8_t *data, uint64_t cie_len, parsed_cie_t *out)
{
    const uint8_t *p   = data;
    const uint8_t *end = data + cie_len;

    memset(out, 0, sizeof(*out));

    /* Version */
    if (p >= end)
        return -1;
    out->version = *p++;

    if (out->version != 1 && out->version != 3)
        return -1;  /* Only versions 1 and 3 are supported */

    /* Augmentation string */
    const char *aug = (const char *)p;
    while (p < end && *p != 0)
        p++;
    if (p >= end)
        return -1;
    p++;  /* Skip NUL */

    /* Code alignment factor */
    out->code_align = decode_uleb128(&p);

    /* Data alignment factor */
    out->data_align = decode_sleb128(&p);

    /* Return address register */
    if (out->version == 1) {
        if (p >= end)
            return -1;
        out->ra_reg = *p++;
    } else {
        out->ra_reg = decode_uleb128(&p);
    }

    /* Default encodings */
    out->fde_encoding  = DW_EH_PE_absptr;
    out->lsda_encoding = DW_EH_PE_omit;
    out->personality    = NULL;
    out->has_augmentation = 0;

    /* Parse augmentation data (if 'z' prefix) */
    if (aug[0] == 'z') {
        out->has_augmentation = 1;
        uint64_t aug_len = decode_uleb128(&p);
        const uint8_t *aug_end = p + aug_len;
        const char *a = aug + 1;  /* Skip 'z' */

        while (*a && p < aug_end) {
            switch (*a) {
            case 'L':
                /* LSDA encoding */
                out->lsda_encoding = *p++;
                break;
            case 'P': {
                /* Personality routine pointer */
                uint8_t per_encoding = *p++;
                uint64_t per_addr = read_encoded_pointer(
                    &p, per_encoding, (uint64_t)p);
                out->personality = (_Unwind_Personality_Fn)per_addr;
                break;
            }
            case 'R':
                /* FDE pointer encoding */
                out->fde_encoding = *p++;
                break;
            case 'S':
                /* Signal handler frame — ignored for now */
                break;
            default:
                /* Unknown augmentation character — skip rest */
                p = aug_end;
                break;
            }
            a++;
        }
        p = aug_end;  /* Ensure we skip any remaining augmentation data */
    }

    /* Remaining bytes are initial instructions */
    if (p < end) {
        out->initial_instructions     = p;
        out->initial_instructions_len = (uint64_t)(end - p);
    } else {
        out->initial_instructions     = NULL;
        out->initial_instructions_len = 0;
    }

    return 0;
}

/* ================================================================== */
/*  FDE lookup — linear scan of .eh_frame                              */
/* ================================================================== */

/**
 * Find the FDE covering a given program counter value.
 *
 * .eh_frame is a sequence of records, each prefixed by:
 *   length (4 bytes; 0xFFFFFFFF = extended 8-byte length)
 *   CIE_id / CIE_pointer (4 bytes)
 *     - CIE: id == 0
 *     - FDE: id == offset back to owning CIE (relative to this field)
 *
 * @param pc   Instruction pointer to look up
 * @param out  Output FDE structure
 * @return     0 if found, -1 if not found
 */
static int find_fde(uint64_t pc, parsed_fde_t *out)
{
    const uint8_t *frame_start = (const uint8_t *)__eh_frame_start;
    const uint8_t *frame_end   = (const uint8_t *)__eh_frame_end;
    const uint8_t *p           = frame_start;

    while (p < frame_end) {
        const uint8_t *record_start = p;

        /* Read 4-byte length */
        if (p + 4 > frame_end)
            break;
        uint32_t length32;
        memcpy(&length32, p, 4);
        p += 4;

        if (length32 == 0)
            break;  /* Terminator */

        uint64_t length;
        if (length32 == 0xFFFFFFFF) {
            /* Extended length (8 bytes) */
            if (p + 8 > frame_end)
                break;
            memcpy(&length, p, 8);
            p += 8;
        } else {
            length = length32;
        }

        const uint8_t *record_data = p;
        const uint8_t *record_end  = p + length;

        if (record_end > frame_end)
            break;

        /* Read CIE_id / CIE_pointer (4 bytes) */
        if (p + 4 > record_end) {
            p = record_end;
            continue;
        }
        uint32_t cie_id;
        memcpy(&cie_id, p, 4);
        p += 4;

        if (cie_id == 0) {
            /* This is a CIE — skip it (we parse CIEs on demand from FDEs) */
            p = record_end;
            continue;
        }

        /*
         * This is an FDE.  cie_id is a byte offset from &cie_id back to
         * the start of the owning CIE record.
         */
        const uint8_t *cie_record = record_data - cie_id;
        if (cie_record < frame_start) {
            p = record_end;
            continue;
        }

        /* Parse the CIE that this FDE references */
        const uint8_t *cp = cie_record;

        /* CIE length */
        uint32_t cie_len32;
        memcpy(&cie_len32, cp, 4);
        cp += 4;

        uint64_t cie_length;
        if (cie_len32 == 0xFFFFFFFF) {
            memcpy(&cie_length, cp, 8);
            cp += 8;
        } else {
            cie_length = cie_len32;
        }

        /* Skip CIE id (4 bytes of 0) */
        cp += 4;

        parsed_cie_t cie;
        if (parse_cie(cp, cie_length - 4, &cie) != 0) {
            p = record_end;
            continue;
        }

        /* Read FDE initial_location and address_range using FDE encoding */
        uint64_t pc_begin = read_encoded_pointer(
            &p, cie.fde_encoding, (uint64_t)(p));
        uint64_t pc_range = read_encoded_pointer(
            &p, cie.fde_encoding & 0x0F, 0);  /* Range uses value encoding only */

        /* Read augmentation data (if CIE has 'z' augmentation) */
        uint64_t lsda = 0;
        if (cie.has_augmentation) {
            uint64_t aug_len = decode_uleb128(&p);
            const uint8_t *aug_end = p + aug_len;

            if (cie.lsda_encoding != DW_EH_PE_omit && aug_len > 0) {
                lsda = read_encoded_pointer(&p, cie.lsda_encoding, (uint64_t)p);
            }
            p = aug_end;
        }

        /* Check if this FDE covers the target PC */
        if (pc >= pc_begin && pc < pc_begin + pc_range) {
            out->cie_ptr          = cie_record;
            out->pc_begin         = pc_begin;
            out->pc_range         = pc_range;
            out->lsda             = lsda;
            out->instructions     = p;
            out->instructions_len = (uint64_t)(record_end - p);
            out->cie              = cie;
            return 0;
        }

        p = record_end;
    }

    return -1;  /* No FDE found for this PC */
}

/* ================================================================== */
/*  CFI instruction execution                                          */
/* ================================================================== */

/**
 * Execute DWARF CFI instructions to compute the register save state
 * at a given code offset within a function.
 *
 * @param instructions  Pointer to the CFI instruction byte stream
 * @param len           Length of the instruction stream
 * @param code_align    Code alignment factor from CIE
 * @param data_align    Data alignment factor from CIE
 * @param target_offset PC offset (from function start) to stop at
 * @param state         Output register save state
 */
static void execute_cfi(const uint8_t *instructions, uint64_t len,
                        uint64_t code_align, int64_t data_align,
                        uint64_t target_offset, reg_state_t *state)
{
    const uint8_t *p   = instructions;
    const uint8_t *end = instructions + len;
    uint64_t loc = 0;  /* Current code offset */

    while (p < end && loc <= target_offset) {
        uint8_t opcode = *p++;
        uint8_t hi2 = opcode & 0xC0;
        uint8_t lo6 = opcode & 0x3F;

        if (hi2 == DW_CFA_advance_loc_hi) {
            /* DW_CFA_advance_loc: delta = lo6 * code_align */
            loc += (uint64_t)lo6 * code_align;
            if (loc > target_offset)
                break;
        } else if (hi2 == DW_CFA_offset_hi) {
            /* DW_CFA_offset: reg = lo6, offset = ULEB128 * data_align */
            unsigned reg = lo6;
            uint64_t off = decode_uleb128(&p);
            if (reg < DWARF_REG_COUNT) {
                state->rules[reg].type  = RULE_OFFSET;
                state->rules[reg].value = (int64_t)(off) * data_align;
            }
        } else if (hi2 == DW_CFA_restore_hi) {
            /* DW_CFA_restore: reg = lo6 — restore to initial state */
            unsigned reg = lo6;
            if (reg < DWARF_REG_COUNT) {
                state->rules[reg].type  = RULE_UNDEFINED;
                state->rules[reg].value = 0;
            }
        } else {
            /* Extended opcodes (hi2 == 0) */
            switch (opcode) {
            case DW_CFA_nop:
                break;

            case DW_CFA_set_loc:
                /*
                 * Absolute code location.  The encoding depends on the CIE
                 * but for simplicity we read a native pointer.
                 */
                if (p + 8 <= end) {
                    memcpy(&loc, p, 8);
                    p += 8;
                }
                break;

            case DW_CFA_advance_loc1:
                if (p < end) {
                    loc += (uint64_t)(*p++) * code_align;
                    if (loc > target_offset)
                        return;
                }
                break;

            case DW_CFA_advance_loc2:
                if (p + 2 <= end) {
                    uint16_t delta;
                    memcpy(&delta, p, 2);
                    p += 2;
                    loc += (uint64_t)delta * code_align;
                    if (loc > target_offset)
                        return;
                }
                break;

            case DW_CFA_advance_loc4:
                if (p + 4 <= end) {
                    uint32_t delta;
                    memcpy(&delta, p, 4);
                    p += 4;
                    loc += (uint64_t)delta * code_align;
                    if (loc > target_offset)
                        return;
                }
                break;

            case DW_CFA_def_cfa: {
                uint64_t reg = decode_uleb128(&p);
                uint64_t off = decode_uleb128(&p);
                state->cfa.reg    = (int)reg;
                state->cfa.offset = (int64_t)off;
                break;
            }

            case DW_CFA_def_cfa_sf: {
                uint64_t reg = decode_uleb128(&p);
                int64_t  off = decode_sleb128(&p);
                state->cfa.reg    = (int)reg;
                state->cfa.offset = off * data_align;
                break;
            }

            case DW_CFA_def_cfa_register: {
                uint64_t reg = decode_uleb128(&p);
                state->cfa.reg = (int)reg;
                break;
            }

            case DW_CFA_def_cfa_offset: {
                uint64_t off = decode_uleb128(&p);
                state->cfa.offset = (int64_t)off;
                break;
            }

            case DW_CFA_def_cfa_offset_sf: {
                int64_t off = decode_sleb128(&p);
                state->cfa.offset = off * data_align;
                break;
            }

            case DW_CFA_offset_extended: {
                uint64_t reg = decode_uleb128(&p);
                uint64_t off = decode_uleb128(&p);
                if (reg < DWARF_REG_COUNT) {
                    state->rules[reg].type  = RULE_OFFSET;
                    state->rules[reg].value = (int64_t)(off) * data_align;
                }
                break;
            }

            case DW_CFA_offset_extended_sf: {
                uint64_t reg = decode_uleb128(&p);
                int64_t  off = decode_sleb128(&p);
                if (reg < DWARF_REG_COUNT) {
                    state->rules[reg].type  = RULE_OFFSET;
                    state->rules[reg].value = off * data_align;
                }
                break;
            }

            case DW_CFA_restore_extended: {
                uint64_t reg = decode_uleb128(&p);
                if (reg < DWARF_REG_COUNT) {
                    state->rules[reg].type  = RULE_UNDEFINED;
                    state->rules[reg].value = 0;
                }
                break;
            }

            case DW_CFA_undefined: {
                uint64_t reg = decode_uleb128(&p);
                if (reg < DWARF_REG_COUNT) {
                    state->rules[reg].type  = RULE_UNDEFINED;
                    state->rules[reg].value = 0;
                }
                break;
            }

            case DW_CFA_same_value: {
                uint64_t reg = decode_uleb128(&p);
                if (reg < DWARF_REG_COUNT) {
                    state->rules[reg].type  = RULE_SAME_VALUE;
                    state->rules[reg].value = 0;
                }
                break;
            }

            case DW_CFA_register: {
                uint64_t reg1 = decode_uleb128(&p);
                uint64_t reg2 = decode_uleb128(&p);
                if (reg1 < DWARF_REG_COUNT) {
                    state->rules[reg1].type  = RULE_REGISTER;
                    state->rules[reg1].value = (int64_t)reg2;
                }
                break;
            }

            case DW_CFA_val_offset: {
                uint64_t reg = decode_uleb128(&p);
                uint64_t off = decode_uleb128(&p);
                if (reg < DWARF_REG_COUNT) {
                    state->rules[reg].type  = RULE_VAL_OFFSET;
                    state->rules[reg].value = (int64_t)(off) * data_align;
                }
                break;
            }

            case DW_CFA_val_offset_sf: {
                uint64_t reg = decode_uleb128(&p);
                int64_t  off = decode_sleb128(&p);
                if (reg < DWARF_REG_COUNT) {
                    state->rules[reg].type  = RULE_VAL_OFFSET;
                    state->rules[reg].value = off * data_align;
                }
                break;
            }

            case DW_CFA_remember_state:
                if (state_stack_top < STATE_STACK_DEPTH) {
                    state_stack[state_stack_top] = *state;
                    state_stack_top++;
                }
                break;

            case DW_CFA_restore_state:
                if (state_stack_top > 0) {
                    state_stack_top--;
                    *state = state_stack[state_stack_top];
                }
                break;

            case DW_CFA_def_cfa_expression: {
                /* Skip DWARF expression block (length-prefixed) */
                uint64_t block_len = decode_uleb128(&p);
                p += block_len;
                break;
            }

            case DW_CFA_expression: {
                /* reg, then expression block */
                decode_uleb128(&p);  /* reg */
                uint64_t block_len = decode_uleb128(&p);
                p += block_len;
                break;
            }

            case DW_CFA_val_expression: {
                decode_uleb128(&p);  /* reg */
                uint64_t block_len = decode_uleb128(&p);
                p += block_len;
                break;
            }

            case DW_CFA_GNU_args_size:
                /* Skip argument size (used by GCC, informational) */
                decode_uleb128(&p);
                break;

            case DW_CFA_GNU_negative_offset_extended: {
                uint64_t reg = decode_uleb128(&p);
                uint64_t off = decode_uleb128(&p);
                if (reg < DWARF_REG_COUNT) {
                    state->rules[reg].type  = RULE_OFFSET;
                    state->rules[reg].value = -((int64_t)(off) * data_align);
                }
                break;
            }

            default:
                /*
                 * Unknown opcode — we cannot safely skip it because we do
                 * not know its operand size.  Stop processing.
                 */
                return;
            }
        }
    }
}

/* ================================================================== */
/*  Register value resolution                                          */
/* ================================================================== */

/**
 * Resolve a register's value based on its save rule and the CFA.
 *
 * @param cursor  Current register state
 * @param reg     DWARF register number to resolve
 * @param rule    Save rule for this register
 * @param cfa     Canonical Frame Address value
 * @return        Resolved register value
 */
static uint64_t resolve_reg(const unwind_cursor_t *cursor, int reg,
                            const reg_rule_t *rule, uint64_t cfa)
{
    switch (rule->type) {
    case RULE_OFFSET: {
        /* Value is stored at CFA + offset */
        uint64_t addr = cfa + (uint64_t)rule->value;
        uint64_t val;
        memcpy(&val, (const void *)addr, 8);
        return val;
    }
    case RULE_VAL_OFFSET:
        /* Value IS CFA + offset (not dereferenced) */
        return cfa + (uint64_t)rule->value;

    case RULE_REGISTER:
        /* Value is in another register */
        if (rule->value >= 0 && rule->value < DWARF_REG_COUNT)
            return cursor->regs[rule->value];
        return 0;

    case RULE_SAME_VALUE:
        /* Register retains its current value */
        if (reg >= 0 && reg < DWARF_REG_COUNT)
            return cursor->regs[reg];
        return 0;

    case RULE_UNDEFINED:
    default:
        return 0;
    }
}

/* ================================================================== */
/*  Single-frame unwinding                                             */
/* ================================================================== */

/**
 * Step the cursor one frame up the call stack.
 *
 * Finds the FDE for cursor->regs[DWARF_RA] (the current IP), executes
 * CFI instructions to determine where the caller's registers are saved,
 * and updates the cursor to represent the caller's frame.
 *
 * @param cursor  In/out cursor representing current frame state
 * @return        0 on success, -1 if no FDE found (end of stack)
 */
static int step_cursor(unwind_cursor_t *cursor)
{
    /*
     * The return address (RA) in the cursor points one past the call
     * instruction.  Subtract 1 so we land inside the call instruction's
     * range — this ensures the FDE lookup matches the calling function.
     */
    uint64_t pc = cursor->regs[DWARF_RA] - 1;

    /* Find the FDE covering this PC */
    parsed_fde_t fde;
    if (find_fde(pc, &fde) != 0)
        return -1;

    /* Initialize register state from CIE initial instructions */
    reg_state_t state;
    memset(&state, 0, sizeof(state));
    state.cfa.reg    = DWARF_RSP;
    state.cfa.offset = 8;  /* Default: CFA = RSP + 8 */

    /* Mark all registers undefined initially */
    for (int i = 0; i < DWARF_REG_COUNT; i++)
        state.rules[i].type = RULE_UNDEFINED;

    state_stack_top = 0;

    /* Execute CIE initial instructions (establish baseline rules) */
    if (fde.cie.initial_instructions && fde.cie.initial_instructions_len > 0) {
        execute_cfi(fde.cie.initial_instructions,
                    fde.cie.initial_instructions_len,
                    fde.cie.code_align, fde.cie.data_align,
                    (uint64_t)-1,  /* Run all initial instructions */
                    &state);
    }

    /* Save initial state (for DW_CFA_restore instructions in FDE) */
    /* Note: we already handle restore as RULE_UNDEFINED above */

    /* Execute FDE instructions up to the target PC offset */
    uint64_t target_offset = pc - fde.pc_begin;
    if (fde.instructions && fde.instructions_len > 0) {
        execute_cfi(fde.instructions, fde.instructions_len,
                    fde.cie.code_align, fde.cie.data_align,
                    target_offset, &state);
    }

    /* Compute the CFA value */
    uint64_t cfa;
    if (state.cfa.reg >= 0 && state.cfa.reg < DWARF_REG_COUNT) {
        cfa = cursor->regs[state.cfa.reg] + (uint64_t)state.cfa.offset;
    } else {
        return -1;
    }

    /* Resolve all register values for the caller's frame */
    unwind_cursor_t new_cursor;
    memset(&new_cursor, 0, sizeof(new_cursor));

    for (int i = 0; i < DWARF_REG_COUNT; i++) {
        if (state.rules[i].type != RULE_UNDEFINED) {
            new_cursor.regs[i] = resolve_reg(cursor, i, &state.rules[i], cfa);
        } else if (i == DWARF_RSP) {
            /* RSP defaults to CFA if not explicitly saved */
            new_cursor.regs[i] = cfa;
        } else {
            new_cursor.regs[i] = cursor->regs[i];
        }
    }

    /* Store metadata from the FDE */
    new_cursor.func_start  = fde.pc_begin;
    new_cursor.lsda        = fde.lsda;
    new_cursor.personality  = fde.cie.personality;

    *cursor = new_cursor;
    return 0;
}

/* ================================================================== */
/*  Itanium ABI — context accessors                                    */
/* ================================================================== */

uint64_t _Unwind_GetGR(struct _Unwind_Context *context, int reg_index)
{
    unwind_cursor_t *cursor = (unwind_cursor_t *)context;
    if (reg_index >= 0 && reg_index < DWARF_REG_COUNT)
        return cursor->regs[reg_index];
    return 0;
}

void _Unwind_SetGR(struct _Unwind_Context *context, int reg_index,
                   uint64_t value)
{
    unwind_cursor_t *cursor = (unwind_cursor_t *)context;
    if (reg_index >= 0 && reg_index < DWARF_REG_COUNT)
        cursor->regs[reg_index] = value;
}

uint64_t _Unwind_GetIP(struct _Unwind_Context *context)
{
    unwind_cursor_t *cursor = (unwind_cursor_t *)context;
    return cursor->regs[DWARF_RA];
}

void _Unwind_SetIP(struct _Unwind_Context *context, uint64_t new_ip)
{
    unwind_cursor_t *cursor = (unwind_cursor_t *)context;
    cursor->regs[DWARF_RA] = new_ip;
}

uint64_t _Unwind_GetLanguageSpecificData(struct _Unwind_Context *context)
{
    unwind_cursor_t *cursor = (unwind_cursor_t *)context;
    return cursor->lsda;
}

uint64_t _Unwind_GetRegionStart(struct _Unwind_Context *context)
{
    unwind_cursor_t *cursor = (unwind_cursor_t *)context;
    return cursor->func_start;
}

uint64_t _Unwind_GetCFA(struct _Unwind_Context *context)
{
    /*
     * Return the stack pointer value, which at the call site equals the
     * CFA (the CFA is defined as RSP at the point of the call instruction
     * plus 8, but by the time we have unwound, cursor->regs[DWARF_RSP]
     * holds the CFA value).
     */
    unwind_cursor_t *cursor = (unwind_cursor_t *)context;
    return cursor->regs[DWARF_RSP];
}

/* ================================================================== */
/*  Itanium ABI — core unwinding entry points                          */
/* ================================================================== */

/**
 * Internal implementation of _Unwind_RaiseException.
 *
 * Called from the ASM trampoline after it has saved the caller's
 * registers into cursor_ptr.  Performs the standard two-phase unwind:
 *
 *   Phase 1: Search for a handler by calling each frame's personality
 *            with _UA_SEARCH_PHASE.
 *   Phase 2: Perform cleanup by calling each frame's personality with
 *            _UA_CLEANUP_PHASE (and _UA_HANDLER_FRAME for the target).
 *
 * @param exception_object  The exception being thrown
 * @param cursor_ptr        Pointer to unwind_cursor_t with saved regs
 * @return                  _URC_END_OF_STACK if no handler found
 *                          (does not return on success — jumps to handler)
 */
_Unwind_Reason_Code _Unwind_RaiseException_impl(
    struct _Unwind_Exception *exception_object,
    void *cursor_ptr)
{
    unwind_cursor_t *initial_cursor = (unwind_cursor_t *)cursor_ptr;

    /* ============================================================== */
    /*  Phase 1: Search for a handler                                  */
    /* ============================================================== */

    unwind_cursor_t phase1 = *initial_cursor;
    uint64_t handler_cfa = 0;

    for (;;) {
        /* Step to the next (caller) frame */
        if (step_cursor(&phase1) != 0)
            return _URC_END_OF_STACK;

        /* If this frame has a personality, ask it */
        if (phase1.personality) {
            _Unwind_Reason_Code result = phase1.personality(
                1,  /* version */
                _UA_SEARCH_PHASE,
                exception_object->exception_class,
                exception_object,
                (struct _Unwind_Context *)&phase1);

            if (result == _URC_HANDLER_FOUND) {
                /*
                 * Record the CFA of the handler frame so Phase 2 can
                 * identify it.  Store it in the exception object's
                 * private fields per the ABI specification.
                 */
                handler_cfa = phase1.regs[DWARF_RSP];
                exception_object->private_1 = 0;  /* Reserved */
                exception_object->private_2 = handler_cfa;
                break;
            }

            if (result != _URC_CONTINUE_UNWIND)
                return _URC_FATAL_PHASE1_ERROR;
        }
    }

    /* ============================================================== */
    /*  Phase 2: Cleanup and transfer to handler                       */
    /* ============================================================== */

    unwind_cursor_t phase2 = *initial_cursor;

    for (;;) {
        if (step_cursor(&phase2) != 0)
            return _URC_FATAL_PHASE2_ERROR;

        if (phase2.personality) {
            _Unwind_Action actions = _UA_CLEANUP_PHASE;

            /* Check if this is the handler frame */
            if (phase2.regs[DWARF_RSP] == handler_cfa)
                actions |= _UA_HANDLER_FRAME;

            _Unwind_Reason_Code result = phase2.personality(
                1,  /* version */
                actions,
                exception_object->exception_class,
                exception_object,
                (struct _Unwind_Context *)&phase2);

            if (result == _URC_INSTALL_CONTEXT) {
                /*
                 * The personality has set up the landing pad address (via
                 * _Unwind_SetIP) and the exception registers (via
                 * _Unwind_SetGR).  Transfer control.
                 */
                _unwind_restore_and_jump(&phase2);
                /* Does not return */
            }

            if (result != _URC_CONTINUE_UNWIND)
                return _URC_FATAL_PHASE2_ERROR;
        }
    }
}

/**
 * Resume Phase 2 unwinding after a cleanup landing pad.
 *
 * This is called by compiler-generated code at the end of a cleanup
 * (a catch-all or destructor call that re-throws).  The cursor state
 * at the point of _Unwind_Resume is established by the landing pad
 * itself — we capture it from the current stack frame.
 */
void _Unwind_Resume(struct _Unwind_Exception *exception_object)
{
    /*
     * Build a cursor representing the current frame.  Since _Unwind_Resume
     * is called from a landing pad, the current RBP/RSP give us enough
     * to start unwinding.
     *
     * We use inline assembly to capture the caller's registers.
     * The caller is the landing pad code generated by the compiler.
     */
    unwind_cursor_t cursor;
    memset(&cursor, 0, sizeof(cursor));

    /* Capture callee-saved registers and stack/instruction pointers */
    register uint64_t rbx_val __asm__("rbx");
    register uint64_t rbp_val __asm__("rbp");
    register uint64_t r12_val __asm__("r12");
    register uint64_t r13_val __asm__("r13");
    register uint64_t r14_val __asm__("r14");
    register uint64_t r15_val __asm__("r15");

    cursor.regs[DWARF_RBX] = rbx_val;
    cursor.regs[DWARF_RBP] = rbp_val;
    cursor.regs[DWARF_R12] = r12_val;
    cursor.regs[DWARF_R13] = r13_val;
    cursor.regs[DWARF_R14] = r14_val;
    cursor.regs[DWARF_R15] = r15_val;

    /* RSP: our caller's stack pointer (past the return address) */
    uint64_t rsp_val;
    __asm__ volatile ("lea 8(%%rsp), %0" : "=r"(rsp_val));
    cursor.regs[DWARF_RSP] = rsp_val;

    /* Return address = our caller's next instruction */
    uint64_t ret_addr;
    __asm__ volatile ("mov (%%rsp), %0" : "=r"(ret_addr));
    cursor.regs[DWARF_RA] = ret_addr;

    uint64_t handler_cfa = exception_object->private_2;

    /* Continue Phase 2 from the current position */
    for (;;) {
        if (step_cursor(&cursor) != 0)
            break;  /* Fatal: end of stack during Phase 2 */

        if (cursor.personality) {
            _Unwind_Action actions = _UA_CLEANUP_PHASE;

            if (cursor.regs[DWARF_RSP] == handler_cfa)
                actions |= _UA_HANDLER_FRAME;

            _Unwind_Reason_Code result = cursor.personality(
                1,
                actions,
                exception_object->exception_class,
                exception_object,
                (struct _Unwind_Context *)&cursor);

            if (result == _URC_INSTALL_CONTEXT) {
                _unwind_restore_and_jump(&cursor);
                /* Does not return */
            }

            if (result != _URC_CONTINUE_UNWIND)
                break;
        }
    }

    /* If we reach here, Phase 2 failed catastrophically */
    __builtin_trap();
}

void _Unwind_DeleteException(struct _Unwind_Exception *exception_object)
{
    if (exception_object && exception_object->exception_cleanup) {
        exception_object->exception_cleanup(
            _URC_FOREIGN_EXCEPTION_CAUGHT,
            exception_object);
    }
}

/* ================================================================== */
/*  Backtrace support                                                  */
/* ================================================================== */

_Unwind_Reason_Code _Unwind_Backtrace(_Unwind_Trace_Fn callback, void *arg)
{
    unwind_cursor_t cursor;
    memset(&cursor, 0, sizeof(cursor));

    /* Capture current register state */
    register uint64_t rbx_val __asm__("rbx");
    register uint64_t rbp_val __asm__("rbp");
    register uint64_t r12_val __asm__("r12");
    register uint64_t r13_val __asm__("r13");
    register uint64_t r14_val __asm__("r14");
    register uint64_t r15_val __asm__("r15");

    cursor.regs[DWARF_RBX] = rbx_val;
    cursor.regs[DWARF_RBP] = rbp_val;
    cursor.regs[DWARF_R12] = r12_val;
    cursor.regs[DWARF_R13] = r13_val;
    cursor.regs[DWARF_R14] = r14_val;
    cursor.regs[DWARF_R15] = r15_val;

    uint64_t rsp_val;
    __asm__ volatile ("lea 8(%%rsp), %0" : "=r"(rsp_val));
    cursor.regs[DWARF_RSP] = rsp_val;

    uint64_t ret_addr;
    __asm__ volatile ("mov (%%rsp), %0" : "=r"(ret_addr));
    cursor.regs[DWARF_RA] = ret_addr;

    /* Walk each frame */
    for (;;) {
        _Unwind_Reason_Code rc = callback(
            (struct _Unwind_Context *)&cursor, arg);
        if (rc != _URC_NO_REASON)
            return rc;

        if (step_cursor(&cursor) != 0)
            return _URC_END_OF_STACK;

        /* Safety check: zero return address means end of call chain */
        if (cursor.regs[DWARF_RA] == 0)
            return _URC_END_OF_STACK;
    }
}
