/// Text changed event — fired by TextField, SearchField, TextArea.
pub struct TextChangedEvent {
    /// The control ID whose text changed.
    pub id: u32,
}

impl TextChangedEvent {
    /// Get the current text content from the control.
    ///
    /// Queries the control's text via FFI. Call from within
    /// the event handler to get the updated text.
    pub fn text(&self) -> alloc::string::String {
        let ctrl = crate::Control::from_id(self.id);
        let mut buf = [0u8; 512];
        let len = ctrl.get_text(&mut buf) as usize;
        let len = len.min(512);
        alloc::string::String::from_utf8_lossy(&buf[..len]).into_owned()
    }
}
