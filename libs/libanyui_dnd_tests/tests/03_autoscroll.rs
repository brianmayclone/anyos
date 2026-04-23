use libanyui_dnd_tests::dnd::*;

#[test]
fn small_viewports_never_auto_scroll() {
    // Edge zone is 24 px on each side; a viewport of 48 px or less has no
    // neutral middle zone, so auto-scroll must be disabled to avoid
    // endlessly oscillating.
    for extent in 0..=48 {
        for pointer in -10..=extent + 10 {
            assert_eq!(
                autoscroll_delta(pointer, extent),
                0,
                "extent={} pointer={}",
                extent,
                pointer
            );
        }
    }
}

#[test]
fn middle_of_viewport_is_neutral() {
    // Anything well away from both edges should not scroll.
    assert_eq!(autoscroll_delta(100, 200), 0);
    assert_eq!(autoscroll_delta(500, 1000), 0);
}

#[test]
fn near_top_returns_negative_delta() {
    let extent = 400;
    let pointer = 3; // deep inside the top edge zone
    let dy = autoscroll_delta(pointer, extent);
    assert!(dy < 0, "expected upward scroll, got {}", dy);
    assert!(dy >= -DND_AUTOSCROLL_STEP, "overshot cap: {}", dy);
}

#[test]
fn near_bottom_returns_positive_delta() {
    let extent = 400;
    let pointer = extent - 3;
    let dy = autoscroll_delta(pointer, extent);
    assert!(dy > 0, "expected downward scroll, got {}", dy);
    assert!(dy <= DND_AUTOSCROLL_STEP);
}

#[test]
fn pointer_outside_viewport_does_not_scroll() {
    // Pointer above / below the viewport — autoscroll should not engage
    // (the caller is responsible for passing window-local coords).
    assert_eq!(autoscroll_delta(-1, 400), 0);
    assert_eq!(autoscroll_delta(401, 400), 0);
}

#[test]
fn ramp_is_monotonic_top_edge() {
    let extent = 500;
    let mut prev = autoscroll_delta(DND_AUTOSCROLL_EDGE, extent); // zero
    for pointer in (0..DND_AUTOSCROLL_EDGE).rev() {
        let dy = autoscroll_delta(pointer, extent);
        assert!(dy <= prev, "non-monotonic: {} -> {}", prev, dy);
        prev = dy;
    }
}

#[test]
fn drag_threshold_constants() {
    assert!(!drag_threshold_exceeded(0, 0));
    assert!(!drag_threshold_exceeded(DND_DRAG_THRESHOLD, DND_DRAG_THRESHOLD));
    assert!(drag_threshold_exceeded(DND_DRAG_THRESHOLD + 1, 0));
    assert!(drag_threshold_exceeded(0, -(DND_DRAG_THRESHOLD + 1)));
    assert!(drag_threshold_exceeded(-(DND_DRAG_THRESHOLD + 1), 0));
}
