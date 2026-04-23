use libanyui_dnd_tests::dnd::*;

// Modifier encoding: bit 0 = Ctrl, bit 1 = Shift.
const NONE: u32 = 0;
const CTRL: u32 = 1;
const SHIFT: u32 = 2;
const CTRL_SHIFT: u32 = 3;

#[test]
fn preferred_effect_ordering() {
    assert_eq!(preferred_effect(DND_EFFECT_ALL), DND_EFFECT_MOVE);
    assert_eq!(
        preferred_effect(DND_EFFECT_COPY | DND_EFFECT_LINK),
        DND_EFFECT_COPY
    );
    assert_eq!(preferred_effect(DND_EFFECT_LINK), DND_EFFECT_LINK);
    assert_eq!(preferred_effect(DND_EFFECT_NONE), DND_EFFECT_NONE);
}

#[test]
fn empty_overlap_returns_none() {
    // Source allows only Copy, target only requests Move -> no overlap.
    let effect = negotiate_effect(DND_EFFECT_COPY, DND_EFFECT_MOVE, NONE);
    assert_eq!(effect, DND_EFFECT_NONE);
}

#[test]
fn single_requested_effect_wins_when_allowed() {
    // Target explicitly asks for Link and the source permits it, no modifiers.
    let effect = negotiate_effect(
        DND_EFFECT_COPY | DND_EFFECT_LINK,
        DND_EFFECT_LINK,
        NONE,
    );
    assert_eq!(effect, DND_EFFECT_LINK);
}

#[test]
fn ctrl_forces_copy_when_allowed() {
    let effect = negotiate_effect(DND_EFFECT_ALL, DND_EFFECT_ALL, CTRL);
    assert_eq!(effect, DND_EFFECT_COPY);
}

#[test]
fn shift_forces_move_when_allowed() {
    let effect = negotiate_effect(DND_EFFECT_ALL, DND_EFFECT_ALL, SHIFT);
    assert_eq!(effect, DND_EFFECT_MOVE);
}

#[test]
fn ctrl_shift_forces_link_when_allowed() {
    let effect = negotiate_effect(DND_EFFECT_ALL, DND_EFFECT_ALL, CTRL_SHIFT);
    assert_eq!(effect, DND_EFFECT_LINK);
}

#[test]
fn forced_effect_is_ignored_when_not_allowed() {
    // Ctrl wants Copy, but source only allows Move. Falls back to the
    // overlap preference (Move wins).
    let effect = negotiate_effect(DND_EFFECT_MOVE, DND_EFFECT_MOVE, CTRL);
    assert_eq!(effect, DND_EFFECT_MOVE);
}

#[test]
fn no_modifiers_prefers_move_when_both_sides_allow_all() {
    let effect = negotiate_effect(DND_EFFECT_ALL, DND_EFFECT_ALL, NONE);
    assert_eq!(effect, DND_EFFECT_MOVE);
}

#[test]
fn effect_label_matches_bit() {
    assert_eq!(effect_label(DND_EFFECT_COPY), "copy");
    assert_eq!(effect_label(DND_EFFECT_MOVE), "move");
    assert_eq!(effect_label(DND_EFFECT_LINK), "link");
    assert_eq!(effect_label(DND_EFFECT_NONE), "none");
    assert_eq!(effect_label(DND_EFFECT_ALL), "multi");
}
