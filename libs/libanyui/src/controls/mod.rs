//! Concrete control types — each in its own file, each "derives from" Control.
//!
//! The `create_control()` factory function creates the right concrete type
//! based on the `ControlKind` discriminator.

use alloc::boxed::Box;
use crate::control::{Control, ControlBase, ControlKind, ControlId};

pub mod window;
pub mod view;
pub mod label;
pub mod button;
pub mod textfield;
pub mod toggle;
pub mod checkbox;
pub mod slider;
pub mod radio_button;
pub mod progress_bar;
pub mod stepper;
pub mod segmented;
pub mod table_view;
pub mod scroll_view;
pub mod sidebar;
pub mod navbar;
pub mod tabbar;
pub mod toolbar;
pub mod card;
pub mod groupbox;
pub mod split_view;
pub mod divider;
pub mod alert;
pub mod context_menu;
pub mod tooltip;
pub mod image_view;
pub mod status_indicator;
pub mod colorwell;
pub mod searchfield;
pub mod textarea;
pub mod icon_button;
pub mod badge;
pub mod tag;

/// Factory: create a concrete control based on `kind`.
///
/// Applies default sizes for kinds that define them (when w or h is 0).
pub fn create_control(
    kind: ControlKind,
    id: ControlId,
    parent: ControlId,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    text: &[u8],
) -> Box<dyn Control> {
    let (dw, dh) = kind.default_size();
    let w = if w == 0 && dw > 0 { dw } else { w };
    let h = if h == 0 && dh > 0 { dh } else { h };

    let base = ControlBase::new(id, parent, x, y, w, h).with_text(text);

    match kind {
        ControlKind::Window => Box::new(window::Window::new(base)),
        ControlKind::View => Box::new(view::View::new(base)),
        ControlKind::Label => Box::new(label::Label::new(base)),
        ControlKind::Button => Box::new(button::Button::new(base)),
        ControlKind::TextField => Box::new(textfield::TextField::new(base)),
        ControlKind::Toggle => Box::new(toggle::Toggle::new(base)),
        ControlKind::Checkbox => Box::new(checkbox::Checkbox::new(base)),
        ControlKind::Slider => Box::new(slider::Slider::new(base)),
        ControlKind::RadioButton => Box::new(radio_button::RadioButton::new(base)),
        ControlKind::ProgressBar => Box::new(progress_bar::ProgressBar::new(base)),
        ControlKind::Stepper => Box::new(stepper::Stepper::new(base)),
        ControlKind::SegmentedControl => Box::new(segmented::SegmentedControl::new(base)),
        ControlKind::TableView => Box::new(table_view::TableView::new(base)),
        ControlKind::ScrollView => Box::new(scroll_view::ScrollView::new(base)),
        ControlKind::Sidebar => Box::new(sidebar::Sidebar::new(base)),
        ControlKind::NavigationBar => Box::new(navbar::NavigationBar::new(base)),
        ControlKind::TabBar => Box::new(tabbar::TabBar::new(base)),
        ControlKind::Toolbar => Box::new(toolbar::Toolbar::new(base)),
        ControlKind::Card => Box::new(card::Card::new(base)),
        ControlKind::GroupBox => Box::new(groupbox::GroupBox::new(base)),
        ControlKind::SplitView => Box::new(split_view::SplitView::new(base)),
        ControlKind::Divider => Box::new(divider::Divider::new(base)),
        ControlKind::Alert => Box::new(alert::Alert::new(base)),
        ControlKind::ContextMenu => Box::new(context_menu::ContextMenu::new(base)),
        ControlKind::Tooltip => Box::new(tooltip::Tooltip::new(base)),
        ControlKind::ImageView => Box::new(image_view::ImageView::new(base)),
        ControlKind::StatusIndicator => Box::new(status_indicator::StatusIndicator::new(base)),
        ControlKind::ColorWell => Box::new(colorwell::ColorWell::new(base)),
        ControlKind::SearchField => Box::new(searchfield::SearchField::new(base)),
        ControlKind::TextArea => Box::new(textarea::TextArea::new(base)),
        ControlKind::IconButton => Box::new(icon_button::IconButton::new(base)),
        ControlKind::Badge => Box::new(badge::Badge::new(base)),
        ControlKind::Tag => Box::new(tag::Tag::new(base)),
    }
}
