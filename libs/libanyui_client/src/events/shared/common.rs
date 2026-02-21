/// Base event args — for events with no additional data (close, resize, focus, blur).
pub struct EventArgs {
    /// The control ID that fired the event.
    pub id: u32,
}
