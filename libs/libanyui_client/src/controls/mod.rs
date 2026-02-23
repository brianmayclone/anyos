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
mod texteditor;
mod treeview;

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
mod filedialog;

// ── Re-exports ──
pub use label::{Label, TEXT_ALIGN_LEFT, TEXT_ALIGN_CENTER, TEXT_ALIGN_RIGHT};
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
pub use iconbutton::{IconButton, ICON_NEW_FILE, ICON_FOLDER_OPEN, ICON_SAVE, ICON_SAVE_ALL,
    ICON_BUILD, ICON_PLAY, ICON_STOP, ICON_SETTINGS, ICON_FILES, ICON_GIT_BRANCH, ICON_SEARCH,
    ICON_REFRESH};
pub use badge::Badge;
pub use tag::Tag;
pub use canvas::Canvas;
pub use datagrid::{DataGrid, ColumnDef, ALIGN_LEFT, ALIGN_CENTER, ALIGN_RIGHT,
    SELECTION_SINGLE, SELECTION_MULTI, SORT_NONE, SORT_ASCENDING, SORT_DESCENDING,
    SORT_STRING, SORT_NUMERIC};
pub use texteditor::TextEditor;
pub use treeview::TreeView;

pub use expander::Expander;
pub use window::{Window, WIN_FLAG_BORDERLESS, WIN_FLAG_NOT_RESIZABLE, WIN_FLAG_ALWAYS_ON_TOP,
    WIN_FLAG_NO_CLOSE, WIN_FLAG_NO_MINIMIZE, WIN_FLAG_NO_MAXIMIZE, WIN_FLAG_SHADOW};
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
pub use filedialog::FileDialog;
