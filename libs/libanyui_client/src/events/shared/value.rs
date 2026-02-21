/// Value changed event — fired by Slider, Stepper, ProgressBar, SplitView.
pub struct ValueChangedEvent {
    /// The control ID whose value changed.
    pub id: u32,
    /// The new value (0-100 for Slider/ProgressBar, arbitrary for Stepper).
    pub value: u32,
}
