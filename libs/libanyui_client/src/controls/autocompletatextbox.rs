// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! AutoCompleteTextField — text input field with autocomplete suggestions.
//!
//! Extends TextField with a suggestions list that can be filtered and displayed
//! in a popup as the user types. Suggestions are pipe-separated strings.

use crate::events::TextChangedEvent;
use crate::{events, lib, Control, Widget, KIND_AUTO_COMPLETE_TEXT_FIELD};
use alloc::string::String;

/// Event fired when a suggestion is selected from the autocomplete popup.
pub struct SuggestionSelectedEvent {
    pub id: u32,
    pub selected_text: String,
}

leaf_control!(AutoCompleteTextField, KIND_AUTO_COMPLETE_TEXT_FIELD);

impl AutoCompleteTextField {
    /// Create a new autocomplete text field.
    pub fn new() -> Self {
        let id = (lib().create_control)(KIND_AUTO_COMPLETE_TEXT_FIELD, core::ptr::null(), 0);
        Self {
            ctrl: Control { id },
        }
    }

    /// Set the placeholder text shown when the field is empty.
    pub fn set_placeholder(&self, text: &str) {
        (lib().textfield_set_placeholder)(self.ctrl.id, text.as_ptr(), text.len() as u32);
    }

    /// Set the suggestions list (pipe-separated, e.g., "Alice <alice@example.com>|Bob <bob@example.com>").
    /// The server will filter these as the user types.
    pub fn set_suggestions(&self, items: &str) {
        (lib().autocomplete_set_suggestions)(self.ctrl.id, items.as_ptr(), items.len() as u32);
    }

    /// Register a callback for when the text changes.
    pub fn on_text_changed(&self, mut f: impl FnMut(&TextChangedEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&TextChangedEvent { id }));
        (lib().on_change_fn)(self.ctrl.id, thunk, ud);
    }

    /// Register a callback for when the user presses Enter.
    pub fn on_submit(&self, mut f: impl FnMut(&crate::events::SubmitEvent) + 'static) {
        let (thunk, ud) = events::register(move |id, _| f(&crate::events::SubmitEvent { id }));
        (lib().on_submit_fn)(self.ctrl.id, thunk, ud);
    }
}
