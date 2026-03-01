//! x86 segmentation address translation.
//!
//! Converts a segment-relative offset into a linear (virtual) address by
//! applying base, limit, and access-rights checks from the cached segment
//! descriptor. The behavior differs across real mode, protected mode, and
//! long mode according to the Intel SDM Vol. 3A Chapter 3.

use crate::error::{Result, VmError};
use crate::registers::SegmentDescriptor;

use super::AccessType;

/// Translate a segment-relative `offset` to a linear address.
///
/// # Modes
///
/// - **Long mode (64-bit):** CS, DS, ES, SS treat the segment base as zero;
///   only FS and GS may have a non-zero base (set via MSR). The segment limit
///   and access-rights checks are disabled.
///
/// - **Protected mode (32-bit/16-bit):** The full segment descriptor is checked
///   for presence, privilege, limit, and access type before computing
///   `base + offset`.
///
/// - **Real mode:** The linear address is simply `base + offset`, capped at
///   the real-mode 64 KiB segment limit. Real mode is detected heuristically
///   by `limit == 0xFFFF` combined with `!big` and `!long_mode`.
///
/// # Parameters
///
/// - `seg`: Cached segment descriptor (hidden part of the segment register).
/// - `offset`: Logical offset within the segment.
/// - `access`: Whether this is a read, write, or instruction fetch.
/// - `cpl`: Current privilege level (0-3).
/// - `long_mode`: Whether the CPU is in 64-bit long mode (EFER.LMA=1 && CS.L=1).
///
/// # Errors
///
/// Returns `VmError::GeneralProtection(0)` on segment access violations and
/// `VmError::StackFault(0)` for SS-related faults.
pub fn segment_translate(
    seg: &SegmentDescriptor,
    offset: u64,
    access: AccessType,
    cpl: u8,
    long_mode: bool,
) -> Result<u64> {
    // ── Long mode: flat segments except FS/GS ──
    if long_mode {
        // In 64-bit mode the segment base is forced to 0 for CS, DS, ES, SS.
        // FS and GS retain their base (typically set via WRMSR to FS_BASE /
        // GS_BASE). No limit or access checks are performed.
        return Ok(seg.base.wrapping_add(offset));
    }

    // ── Real mode heuristic ──
    // A real-mode descriptor has limit=0xFFFF, no big flag, and no long_mode
    // flag. In real mode the only check is the 64 KiB segment size.
    if seg.limit == 0xFFFF && !seg.big && !seg.granularity {
        let linear = seg.base.wrapping_add(offset);
        if offset > 0xFFFF {
            return Err(VmError::GeneralProtection(0));
        }
        return Ok(linear);
    }

    // ── Protected mode ──

    // Segment must be present.
    if !seg.present {
        return Err(VmError::SegmentNotPresent(seg.selector as u32));
    }

    // Privilege check (non-conforming code or data segments).
    // For non-conforming code/data: max(CPL, RPL) must be <= DPL.
    // For conforming code: CPL >= DPL (any higher or equal privilege).
    if seg.is_code && !seg.is_conforming {
        let rpl = seg.selector & 0x03;
        let eff = if cpl > (rpl as u8) { cpl } else { rpl as u8 };
        if eff > seg.dpl {
            return Err(VmError::GeneralProtection(0));
        }
    } else if !seg.is_code {
        // Data segment privilege check.
        let rpl = seg.selector & 0x03;
        let eff = if cpl > (rpl as u8) { cpl } else { rpl as u8 };
        if eff > seg.dpl {
            return Err(VmError::GeneralProtection(0));
        }
    }
    // Conforming code: CPL >= DPL (always allowed for same or lower privilege).

    // Access-type checks.
    match access {
        AccessType::Execute => {
            if !seg.is_code {
                // Cannot execute a data segment.
                return Err(VmError::GeneralProtection(0));
            }
        }
        AccessType::Write => {
            if seg.is_code || !seg.writable {
                // Cannot write to a code segment or a read-only data segment.
                return Err(VmError::GeneralProtection(0));
            }
        }
        AccessType::Read => {
            if seg.is_code && !seg.readable {
                // Execute-only code segment — cannot read.
                return Err(VmError::GeneralProtection(0));
            }
        }
    }

    // Limit check. The offset must not exceed the segment limit.
    // For expand-down data segments (not currently modeled) the logic would
    // be inverted, but real OSes almost never use expand-down segments.
    if offset > seg.limit as u64 {
        // SS violations use #SS, others use #GP.
        // We approximate by checking if the descriptor looks like an SS
        // descriptor (writable, not code).
        if seg.writable && !seg.is_code {
            return Err(VmError::StackFault(0));
        }
        return Err(VmError::GeneralProtection(0));
    }

    Ok(seg.base.wrapping_add(offset))
}
