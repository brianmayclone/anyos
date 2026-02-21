//! Concrete control types — each in its own file, each "derives from" Control.
//!
//! The `create_control()` factory function creates the right concrete type
//! based on the `ControlKind` discriminator.

use alloc::boxed::Box;
use crate::control::{Control, ControlBase, TextControlBase, ControlKind, ControlId};

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
pub mod stack_panel;
pub mod flow_panel;
pub mod table_layout;
pub mod canvas;
pub mod expander;

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

    let base = ControlBase::new(id, parent, x, y, w, h);

    match kind {
        // Non-text controls — take ControlBase directly (text param ignored)
        ControlKind::Window => Box::new(window::Window::new(base)),
        ControlKind::View => Box::new(view::View::new(base)),
        ControlKind::Slider => Box::new(slider::Slider::new(base)),
        ControlKind::ProgressBar => Box::new(progress_bar::ProgressBar::new(base)),
        ControlKind::TableView => Box::new(table_view::TableView::new(base)),
        ControlKind::ScrollView => Box::new(scroll_view::ScrollView::new(base)),
        ControlKind::Sidebar => Box::new(sidebar::Sidebar::new(base)),
        ControlKind::Toolbar => Box::new(toolbar::Toolbar::new(base)),
        ControlKind::Card => Box::new(card::Card::new(base)),
        ControlKind::SplitView => Box::new(split_view::SplitView::new(base)),
        ControlKind::Divider => Box::new(divider::Divider::new(base)),
        ControlKind::ContextMenu => Box::new(context_menu::ContextMenu::new(TextControlBase::new(base).with_text(text))),
        ControlKind::ImageView => Box::new(image_view::ImageView::new(base)),
        ControlKind::ColorWell => Box::new(colorwell::ColorWell::new(base)),
        ControlKind::StackPanel => Box::new(stack_panel::StackPanel::new(base)),
        ControlKind::FlowPanel => Box::new(flow_panel::FlowPanel::new(base)),
        ControlKind::TableLayout => Box::new(table_layout::TableLayout::new(base)),
        ControlKind::Canvas => Box::new(canvas::Canvas::new(base)),

        // Text controls — wrap ControlBase in TextControlBase with text
        ControlKind::Label => Box::new(label::Label::new(TextControlBase::new(base).with_text(text))),
        ControlKind::Button => Box::new(button::Button::new(TextControlBase::new(base).with_text(text))),
        ControlKind::TextField => Box::new(textfield::TextField::new(TextControlBase::new(base).with_text(text))),
        ControlKind::Toggle => Box::new(toggle::Toggle::new(TextControlBase::new(base).with_text(text))),
        ControlKind::Checkbox => Box::new(checkbox::Checkbox::new(TextControlBase::new(base).with_text(text))),
        ControlKind::RadioButton => Box::new(radio_button::RadioButton::new(TextControlBase::new(base).with_text(text))),
        ControlKind::Stepper => Box::new(stepper::Stepper::new(TextControlBase::new(base).with_text(text))),
        ControlKind::SegmentedControl => Box::new(segmented::SegmentedControl::new(TextControlBase::new(base).with_text(text))),
        ControlKind::NavigationBar => Box::new(navbar::NavigationBar::new(TextControlBase::new(base).with_text(text))),
        ControlKind::TabBar => Box::new(tabbar::TabBar::new(TextControlBase::new(base).with_text(text))),
        ControlKind::GroupBox => Box::new(groupbox::GroupBox::new(TextControlBase::new(base).with_text(text))),
        ControlKind::Alert => Box::new(alert::Alert::new(TextControlBase::new(base).with_text(text))),
        ControlKind::Tooltip => Box::new(tooltip::Tooltip::new(TextControlBase::new(base).with_text(text))),
        ControlKind::SearchField => Box::new(searchfield::SearchField::new(TextControlBase::new(base).with_text(text))),
        ControlKind::TextArea => Box::new(textarea::TextArea::new(TextControlBase::new(base).with_text(text))),
        ControlKind::Expander => Box::new(expander::Expander::new(TextControlBase::new(base).with_text(text))),
        ControlKind::IconButton => Box::new(icon_button::IconButton::new(TextControlBase::new(base).with_text(text))),
        ControlKind::Badge => Box::new(badge::Badge::new(TextControlBase::new(base).with_text(text))),
        ControlKind::Tag => Box::new(tag::Tag::new(TextControlBase::new(base).with_text(text))),
        ControlKind::StatusIndicator => Box::new(status_indicator::StatusIndicator::new(TextControlBase::new(base).with_text(text))),
    }
}
