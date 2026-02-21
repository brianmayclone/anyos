//! Shared event types used by multiple controls.

mod click;
mod checked;
mod common;
mod scroll;
mod selection;
mod text;
mod value;

pub use click::ClickEvent;
pub use checked::CheckedChangedEvent;
pub use common::EventArgs;
pub use scroll::ScrollChangedEvent;
pub use selection::SelectionChangedEvent;
pub use text::TextChangedEvent;
pub use value::ValueChangedEvent;
