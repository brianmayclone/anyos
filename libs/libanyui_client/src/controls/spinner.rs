use crate::{lib, Control, Widget, KIND_SPINNER};

leaf_control!(Spinner, KIND_SPINNER);

impl Spinner {
    /// Create a new spinner control (initially stopped).
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_SPINNER, core::ptr::null(), 0);
        Self {
            ctrl: Control { id },
        }
    }

    /// Start the spinner animation.
    /// Returns a timer ID that can be passed to `stop()`.
    pub fn start(&self) -> u32 {
        let ctrl_id = self.ctrl.id;
        let mut frame: u32 = 0;
        crate::set_timer(60, move || {
            frame = frame.wrapping_add(1);
            (lib().set_state)(ctrl_id, frame);
        })
    }

    /// Stop the spinner animation.
    pub fn stop(timer_id: u32) {
        crate::kill_timer(timer_id);
    }

    /// Set the spinner color (ARGB). Pass 0 to use the theme accent color.
    pub fn set_spinner_color(&self, color: u32) {
        self.set_color(color);
    }
}
