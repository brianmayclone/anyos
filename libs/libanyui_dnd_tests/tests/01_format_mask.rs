use libanyui_dnd_tests::dnd::*;

#[test]
fn empty_mask_accepts_nothing() {
    assert!(!format_mask_contains(0, DND_FORMAT_TEXT));
    assert!(!format_mask_contains(0, DND_FORMAT_URI_LIST));
    assert!(!format_mask_contains(0, DND_FORMAT_FILES));
}

#[test]
fn accept_any_accepts_all_nonzero_formats() {
    assert!(format_mask_contains(DND_FORMAT_ACCEPT_ANY, DND_FORMAT_TEXT));
    assert!(format_mask_contains(DND_FORMAT_ACCEPT_ANY, DND_FORMAT_URI_LIST));
    assert!(format_mask_contains(DND_FORMAT_ACCEPT_ANY, DND_FORMAT_FILES));
    assert!(format_mask_contains(DND_FORMAT_ACCEPT_ANY, DND_FORMAT_CUSTOM + 7));
    // But the sentinel explicitly refuses the NONE format.
    assert!(!format_mask_contains(DND_FORMAT_ACCEPT_ANY, DND_FORMAT_NONE));
}

#[test]
fn single_format_mask_matches_only_that_format() {
    let mask = format_mask(DND_FORMAT_FILES);
    assert!(format_mask_contains(mask, DND_FORMAT_FILES));
    assert!(!format_mask_contains(mask, DND_FORMAT_TEXT));
    assert!(!format_mask_contains(mask, DND_FORMAT_URI_LIST));
}

#[test]
fn combined_mask_matches_each_component() {
    let mask = format_mask(DND_FORMAT_TEXT) | format_mask(DND_FORMAT_URI_LIST);
    assert!(format_mask_contains(mask, DND_FORMAT_TEXT));
    assert!(format_mask_contains(mask, DND_FORMAT_URI_LIST));
    assert!(!format_mask_contains(mask, DND_FORMAT_FILES));
}

#[test]
fn custom_format_survives_mask_roundtrip() {
    // Custom formats beyond the framework range should still work — the
    // mask wraps modulo 32 so collisions between distant custom values are
    // possible, but the same value always masks to itself.
    let custom = DND_FORMAT_CUSTOM + 3;
    let mask = format_mask(custom);
    assert!(format_mask_contains(mask, custom));
}

#[test]
fn none_format_never_matches_any_mask() {
    assert!(!format_mask_contains(format_mask(DND_FORMAT_TEXT), DND_FORMAT_NONE));
    assert!(!format_mask_contains(format_mask(DND_FORMAT_FILES), DND_FORMAT_NONE));
    // ACCEPT_ANY is the explicit exception — already covered above.
}
