//! Drag & Drop — generic framework primitives.
//!
//! Pure logic (no global state, no FFI), so host-native test crates can
//! link against this module directly and exercise the core semantics:
//! payload format matching, effect negotiation, and edge-proximity
//! auto-scroll math.
//!
//! Runtime integration (session state, event dispatch, visual feedback,
//! cursor changes) lives in `lib.rs` + `event_loop.rs` and consumes these
//! helpers.

// ── Payload formats ─────────────────────────────────────────────────────
//
// Formats are small integers. Values below `DND_FORMAT_CUSTOM` are
// framework-reserved MIME-style types. Application payloads pick any
// value `>= DND_FORMAT_CUSTOM` and coordinate the meaning between source
// and target.

pub const DND_FORMAT_NONE: u32 = 0;
/// UTF-8 plain text.
pub const DND_FORMAT_TEXT: u32 = 1;
/// Newline-separated `file://` URIs.
pub const DND_FORMAT_URI_LIST: u32 = 2;
/// Newline-separated absolute filesystem paths.
pub const DND_FORMAT_FILES: u32 = 3;
/// Application-defined start. Add offsets for subtypes.
pub const DND_FORMAT_CUSTOM: u32 = 0x1000;

/// Build a format mask accepting a single format.
pub const fn format_mask(fmt: u32) -> u32 {
    // Framework formats map 1:1 to bits; custom formats map modulo 32 to keep
    // the mask as a plain u32. Apps that need more than 32 distinct custom
    // formats can filter inside their own on_drag_enter handler.
    if fmt == 0 {
        0
    } else {
        1u32 << (fmt & 31)
    }
}

/// Returns true if `fmt` is in the acceptance `mask` (built via `format_mask`).
/// A `mask` of `DND_FORMAT_ACCEPT_ANY` accepts any non-zero format.
pub const fn format_mask_contains(mask: u32, fmt: u32) -> bool {
    if mask == DND_FORMAT_ACCEPT_ANY {
        fmt != DND_FORMAT_NONE
    } else {
        (mask & format_mask(fmt)) != 0
    }
}

/// Sentinel mask that accepts any format. Distinct from `!0` so apps can
/// still opt in explicitly.
pub const DND_FORMAT_ACCEPT_ANY: u32 = 0xFFFF_FFFF;

// ── Drop effects ────────────────────────────────────────────────────────

pub const DND_EFFECT_NONE: u32 = 0;
pub const DND_EFFECT_COPY: u32 = 1;
pub const DND_EFFECT_MOVE: u32 = 2;
pub const DND_EFFECT_LINK: u32 = 4;
pub const DND_EFFECT_ALL: u32 =
    DND_EFFECT_COPY | DND_EFFECT_MOVE | DND_EFFECT_LINK;

/// Preference order when the source allows multiple effects and the target
/// does not pick one explicitly: Move > Copy > Link.
pub const fn preferred_effect(allowed: u32) -> u32 {
    if (allowed & DND_EFFECT_MOVE) != 0 {
        DND_EFFECT_MOVE
    } else if (allowed & DND_EFFECT_COPY) != 0 {
        DND_EFFECT_COPY
    } else if (allowed & DND_EFFECT_LINK) != 0 {
        DND_EFFECT_LINK
    } else {
        DND_EFFECT_NONE
    }
}

/// Negotiate the final effect given what the source allows, what the target
/// requested, and the keyboard modifier state.
///
/// Semantics:
/// - If the target explicitly requested a specific effect bit (only one set),
///   that wins iff the source allows it.
/// - Otherwise, Ctrl forces Copy, Shift forces Move, Ctrl+Shift forces Link
///   (when allowed); these match platform convention.
/// - Falls back to `preferred_effect(allowed & requested)`.
///
/// Returns `DND_EFFECT_NONE` if no overlap exists.
pub fn negotiate_effect(allowed: u32, requested: u32, modifiers_ctrl_shift: u32) -> u32 {
    // modifiers_ctrl_shift: bit0 = Ctrl, bit1 = Shift.
    let overlap = allowed & requested;
    if overlap == DND_EFFECT_NONE {
        return DND_EFFECT_NONE;
    }

    let ctrl = (modifiers_ctrl_shift & 1) != 0;
    let shift = (modifiers_ctrl_shift & 2) != 0;

    let forced = match (ctrl, shift) {
        (true, true) => DND_EFFECT_LINK,
        (true, false) => DND_EFFECT_COPY,
        (false, true) => DND_EFFECT_MOVE,
        (false, false) => DND_EFFECT_NONE,
    };

    if forced != DND_EFFECT_NONE && (overlap & forced) != 0 {
        return forced;
    }

    // If the target requested exactly one effect, honour it when allowed.
    if requested.is_power_of_two() && (allowed & requested) != 0 {
        return requested;
    }

    preferred_effect(overlap)
}

/// Decompose an effect bitmask into a stable short label for status bars /
/// debug output. Returns `"none"` if no bits are set.
pub fn effect_label(effect: u32) -> &'static str {
    match effect {
        DND_EFFECT_COPY => "copy",
        DND_EFFECT_MOVE => "move",
        DND_EFFECT_LINK => "link",
        DND_EFFECT_NONE => "none",
        _ => "multi",
    }
}

// ── Auto-scroll math ────────────────────────────────────────────────────

/// Edge zone inside a scrollable viewport that triggers auto-scroll during
/// drag. Pointer positions within this many logical pixels of an edge
/// produce a non-zero scroll delta.
pub const DND_AUTOSCROLL_EDGE: i32 = 24;

/// Maximum scroll step per tick (logical pixels). Tuned to feel
/// responsive without overshooting small lists.
pub const DND_AUTOSCROLL_STEP: i32 = 16;

/// Compute a scroll delta for one axis given the pointer position relative
/// to the viewport and the viewport extent. Positive = scroll forward
/// (right/down), negative = scroll backward (left/up). `0` when outside the
/// edge zone.
pub fn autoscroll_delta(pointer: i32, extent: i32) -> i32 {
    if extent <= DND_AUTOSCROLL_EDGE * 2 {
        return 0;
    }
    if pointer < 0 || pointer > extent {
        return 0;
    }
    if pointer < DND_AUTOSCROLL_EDGE {
        // Stronger pull as the pointer approaches the edge.
        let depth = DND_AUTOSCROLL_EDGE - pointer;
        -scale_step(depth)
    } else if pointer > extent - DND_AUTOSCROLL_EDGE {
        let depth = pointer - (extent - DND_AUTOSCROLL_EDGE);
        scale_step(depth)
    } else {
        0
    }
}

fn scale_step(depth: i32) -> i32 {
    if depth <= 0 {
        return 0;
    }
    // Linear ramp across the 24-pixel edge zone.
    let step = (depth * DND_AUTOSCROLL_STEP) / DND_AUTOSCROLL_EDGE;
    if step < 2 { 2 } else if step > DND_AUTOSCROLL_STEP { DND_AUTOSCROLL_STEP } else { step }
}

// ── Drag threshold ──────────────────────────────────────────────────────

/// Number of logical pixels the pointer must move after mouse-down before a
/// drag session is initiated. Matches the hard-coded value previously in
/// `event_loop::maybe_begin_drag`; centralised so tests can assert it.
pub const DND_DRAG_THRESHOLD: i32 = 4;

/// Returns true when a press-to-current-pointer delta exceeds the drag
/// threshold on either axis.
pub const fn drag_threshold_exceeded(dx: i32, dy: i32) -> bool {
    let ax = if dx < 0 { -dx } else { dx };
    let ay = if dy < 0 { -dy } else { dy };
    ax > DND_DRAG_THRESHOLD || ay > DND_DRAG_THRESHOLD
}
