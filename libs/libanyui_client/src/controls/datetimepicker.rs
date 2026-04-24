use crate::events::ValueChangedEvent;
use crate::{events, lib, Control, Widget, KIND_DATE_TIME_PICKER};

// ── DateTimePicker (DD.MM.YYYY HH:MM) ─────────────────────────────

leaf_control!(DateTimePicker, KIND_DATE_TIME_PICKER);

impl DateTimePicker {
    pub fn new() -> Self {
        let mode = b"datetime";
        let id = (lib().create_control)(KIND_DATE_TIME_PICKER, mode.as_ptr(), mode.len() as u32);
        Self {
            ctrl: Control { id },
        }
    }

    /// Set the date and time.
    pub fn set_datetime(&self, day: u32, month: u32, year: u32, hour: u32, minute: u32) {
        self.ctrl.set_state(pack(year, month, day, hour, minute));
    }

    /// Get the packed value. Use `unpack()` to extract fields.
    pub fn get_value(&self) -> u32 {
        self.ctrl.get_state()
    }

    pub fn day(&self) -> u32 {
        let (_, _, d, _, _) = unpack(self.ctrl.get_state());
        d
    }
    pub fn month(&self) -> u32 {
        let (_, m, _, _, _) = unpack(self.ctrl.get_state());
        m
    }
    pub fn year(&self) -> u32 {
        let (y, _, _, _, _) = unpack(self.ctrl.get_state());
        y
    }
    pub fn hour(&self) -> u32 {
        let (_, _, _, h, _) = unpack(self.ctrl.get_state());
        h
    }
    pub fn minute(&self) -> u32 {
        let (_, _, _, _, m) = unpack(self.ctrl.get_state());
        m
    }

    pub fn on_changed(&self, mut f: impl FnMut(&ValueChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let value = Control::from_id(id).get_state();
            f(&ValueChangedEvent { id, value });
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }
}

// ── DatePicker (DD.MM.YYYY) ────────────────────────────────────────

leaf_control!(DatePicker, KIND_DATE_TIME_PICKER);

impl DatePicker {
    pub fn new() -> Self {
        let mode = b"date";
        let id = (lib().create_control)(KIND_DATE_TIME_PICKER, mode.as_ptr(), mode.len() as u32);
        Self {
            ctrl: Control { id },
        }
    }

    pub fn set_date(&self, day: u32, month: u32, year: u32) {
        self.ctrl.set_state(pack(year, month, day, 0, 0));
    }

    pub fn day(&self) -> u32 {
        let (_, _, d, _, _) = unpack(self.ctrl.get_state());
        d
    }
    pub fn month(&self) -> u32 {
        let (_, m, _, _, _) = unpack(self.ctrl.get_state());
        m
    }
    pub fn year(&self) -> u32 {
        let (y, _, _, _, _) = unpack(self.ctrl.get_state());
        y
    }

    pub fn on_changed(&self, mut f: impl FnMut(&ValueChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let value = Control::from_id(id).get_state();
            f(&ValueChangedEvent { id, value });
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }
}

// ── TimePicker (HH:MM) ────────────────────────────────────────────

leaf_control!(TimePicker, KIND_DATE_TIME_PICKER);

impl TimePicker {
    pub fn new() -> Self {
        let mode = b"time";
        let id = (lib().create_control)(KIND_DATE_TIME_PICKER, mode.as_ptr(), mode.len() as u32);
        Self {
            ctrl: Control { id },
        }
    }

    pub fn set_time(&self, hour: u32, minute: u32) {
        self.ctrl.set_state(pack(0, 0, 0, hour, minute));
    }

    pub fn hour(&self) -> u32 {
        let (_, _, _, h, _) = unpack(self.ctrl.get_state());
        h
    }
    pub fn minute(&self) -> u32 {
        let (_, _, _, _, m) = unpack(self.ctrl.get_state());
        m
    }

    pub fn on_changed(&self, mut f: impl FnMut(&ValueChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| {
            let value = Control::from_id(id).get_state();
            f(&ValueChangedEvent { id, value });
        });
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }
}

// ── Shared pack/unpack helpers ─────────────────────────────────────

/// Pack date/time fields into a u32.
///
/// | Bits  | Field  | Range   |
/// |-------|--------|---------|
/// | 0–5   | minute | 0–59    |
/// | 6–10  | hour   | 0–23    |
/// | 11–15 | day    | 1–31    |
/// | 16–19 | month  | 1–12    |
/// | 20–31 | year   | 0–4095  |
pub fn pack(year: u32, month: u32, day: u32, hour: u32, minute: u32) -> u32 {
    (minute & 0x3F)
        | ((hour & 0x1F) << 6)
        | ((day & 0x1F) << 11)
        | ((month & 0x0F) << 16)
        | ((year & 0xFFF) << 20)
}

/// Unpack a u32 into (year, month, day, hour, minute).
pub fn unpack(v: u32) -> (u32, u32, u32, u32, u32) {
    let minute = v & 0x3F;
    let hour = (v >> 6) & 0x1F;
    let day = (v >> 11) & 0x1F;
    let month = (v >> 16) & 0x0F;
    let year = (v >> 20) & 0xFFF;
    (year, month, day, hour, minute)
}
