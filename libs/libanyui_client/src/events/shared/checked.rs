/// Checked changed event — fired by Toggle, Checkbox, RadioButton.
pub struct CheckedChangedEvent {
    /// The control ID whose checked state changed.
    pub id: u32,
    /// Whether the control is now checked/on.
    pub checked: bool,
}
