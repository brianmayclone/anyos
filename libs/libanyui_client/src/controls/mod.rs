//! Individual control type modules — one file per control.

// ── Leaf controls (no children) ──
mod autocompletatextbox;
mod badge;
mod button;
mod canvas;
mod checkbox;
mod colorpickerdialog;
mod colorwell;
mod combobox;
mod datagrid;
mod datetimepicker;
mod divider;
mod dropdown;
mod iconbutton;
mod imagebutton;
mod imageview;
mod label;
mod linklabel;
mod listbox;
mod menubar;
mod plainbutton;
mod progressbar;
mod radiobutton;
mod searchfield;
mod segmented;
mod slider;
mod spinner;
mod statusindicator;
mod stepper;
mod tag;
mod textarea;
mod texteditor;
mod textfield;
mod toggle;
mod trayicon;
mod treeview;

// ── Container controls (can have children) ──
mod alert;
mod antialiasfiltercontainer;
mod card;
mod contextmenu;
mod expander;
mod flowpanel;
mod groupbox;
mod navigationbar;
mod radiogroup;
mod scrollview;
mod sidebar;
mod splitview;
mod stackpanel;
mod tabbar;
mod tablelayout;
mod tableview;
mod toolbar;
mod tooltip;
mod view;
mod window;

// ── Static dialogs ──
mod filedialog;
mod messagebox;

// ── Re-exports ──
pub use badge::Badge;
pub use button::Button;
pub use canvas::Canvas;
pub use checkbox::Checkbox;
pub use colorpickerdialog::ColorPickerDialog;
pub use colorwell::ColorWell;
pub use datagrid::{
    ColumnDef, DataGrid, ALIGN_CENTER, ALIGN_LEFT, ALIGN_RIGHT, SELECTION_MULTI, SELECTION_SINGLE,
    SORT_ASCENDING, SORT_DESCENDING, SORT_NONE, SORT_NUMERIC, SORT_STRING,
};
pub use divider::Divider;
pub use iconbutton::{
    IconButton, ICON_BUILD, ICON_FILES, ICON_FOLDER_OPEN, ICON_GIT_BRANCH, ICON_NEW_FILE,
    ICON_PLAY, ICON_REFRESH, ICON_SAVE, ICON_SAVE_ALL, ICON_SEARCH, ICON_SETTINGS, ICON_STOP,
};
pub use imagebutton::ImageButton;
pub use imageview::{ImageView, SCALE_FILL, SCALE_FIT, SCALE_NONE, SCALE_STRETCH};
pub use label::{Label, TEXT_ALIGN_CENTER, TEXT_ALIGN_LEFT, TEXT_ALIGN_RIGHT};
pub use linklabel::LinkLabel;
pub use plainbutton::PlainButton;
pub use progressbar::ProgressBar;
pub use radiobutton::RadioButton;
pub use searchfield::SearchField;
pub use segmented::SegmentedControl;
pub use slider::Slider;
pub use spinner::Spinner;
pub use statusindicator::StatusIndicator;
pub use stepper::Stepper;
pub use tag::Tag;
pub use textarea::TextArea;
pub use texteditor::TextEditor;
pub use textfield::TextField;
pub use toggle::Toggle;
pub use tooltip::Tooltip;
pub use treeview::{TreeView, STYLE_BOLD, STYLE_NORMAL};

pub use alert::Alert;
pub use antialiasfiltercontainer::AntiAliasFilterContainer;
pub use autocompletatextbox::AutoCompleteTextField;
pub use card::Card;
pub use combobox::ComboBox;
pub use contextmenu::ContextMenu;
pub use datetimepicker::{
    pack as datetime_pack, unpack as datetime_unpack, DatePicker, DateTimePicker, TimePicker,
};
pub use dropdown::DropDown;
pub use expander::Expander;
pub use flowpanel::FlowPanel;
pub use groupbox::GroupBox;
pub use listbox::ListBox;
pub use navigationbar::NavigationBar;
pub use radiogroup::RadioGroup;
pub use scrollview::ScrollView;
pub use sidebar::Sidebar;
pub use splitview::SplitView;
pub use stackpanel::StackPanel;
pub use tabbar::TabBar;
pub use tablelayout::TableLayout;
pub use tableview::TableView;
pub use toolbar::Toolbar;
pub use view::View;
pub use window::{
    Window, WIN_FLAG_ALPHA_HIT_TEST, WIN_FLAG_ALWAYS_ON_TOP, WIN_FLAG_BORDERLESS,
    WIN_FLAG_NOT_RESIZABLE, WIN_FLAG_NO_CLOSE, WIN_FLAG_NO_MAXIMIZE, WIN_FLAG_NO_MINIMIZE,
    WIN_FLAG_SHADOW,
};

pub use filedialog::FileDialog;
pub use menubar::{
    MenuBar, MenuBarBuilder, MenuBuilder, MenuItemEvent, MENU_FLAG_CHECKED, MENU_FLAG_DISABLED,
    MENU_FLAG_SEPARATOR,
};
pub use messagebox::{MessageBox, MessageBoxType};
pub use trayicon::TrayIcon;
