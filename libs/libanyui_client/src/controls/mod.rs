//! Individual control type modules — one file per control.

// ── Leaf controls (no children) ──
mod label;
mod button;
mod textfield;
mod toggle;
mod checkbox;
mod slider;
mod radiobutton;
mod progressbar;
mod stepper;
mod segmented;
mod divider;
mod imageview;
mod statusindicator;
mod colorwell;
mod searchfield;
mod textarea;
mod iconbutton;
mod badge;
mod tag;
mod canvas;
mod datagrid;

// ── Container controls (can have children) ──
mod expander;
mod tooltip;
mod window;
mod view;
mod card;
mod groupbox;
mod splitview;
mod scrollview;
mod sidebar;
mod navigationbar;
mod tabbar;
mod toolbar;
mod alert;
mod contextmenu;
mod tableview;
mod stackpanel;
mod flowpanel;
mod tablelayout;

// ── Static dialogs ──
mod messagebox;

// ── Re-exports ──
pub use label::Label;
pub use button::Button;
pub use textfield::TextField;
pub use toggle::Toggle;
pub use checkbox::Checkbox;
pub use slider::Slider;
pub use radiobutton::RadioButton;
pub use progressbar::ProgressBar;
pub use stepper::Stepper;
pub use segmented::SegmentedControl;
pub use divider::Divider;
pub use tooltip::Tooltip;
pub use imageview::ImageView;
pub use statusindicator::StatusIndicator;
pub use colorwell::ColorWell;
pub use searchfield::SearchField;
pub use textarea::TextArea;
pub use iconbutton::IconButton;
pub use badge::Badge;
pub use tag::Tag;
pub use canvas::Canvas;
pub use datagrid::{DataGrid, ColumnDef};

pub use expander::Expander;
pub use window::Window;
pub use view::View;
pub use card::Card;
pub use groupbox::GroupBox;
pub use splitview::SplitView;
pub use scrollview::ScrollView;
pub use sidebar::Sidebar;
pub use navigationbar::NavigationBar;
pub use tabbar::TabBar;
pub use toolbar::Toolbar;
pub use alert::Alert;
pub use contextmenu::ContextMenu;
pub use tableview::TableView;
pub use stackpanel::StackPanel;
pub use flowpanel::FlowPanel;
pub use tablelayout::TableLayout;

pub use messagebox::{MessageBox, MessageBoxType};
