use libanyui_client as ui;

#[derive(Clone, Copy, Debug)]
pub struct ToolboxControl {
    pub name: &'static str,
    pub category: &'static str,
}

pub const TOOLBOX_CONTROLS: &[ToolboxControl] = &[
    ToolboxControl {
        name: "Pointer",
        category: "General",
    },
    ToolboxControl {
        name: "Alert",
        category: "Feedback",
    },
    ToolboxControl {
        name: "Badge",
        category: "Feedback",
    },
    ToolboxControl {
        name: "StatusIndicator",
        category: "Feedback",
    },
    ToolboxControl {
        name: "Spinner",
        category: "Feedback",
    },
    ToolboxControl {
        name: "Tag",
        category: "Feedback",
    },
    ToolboxControl {
        name: "Tooltip",
        category: "Feedback",
    },
    ToolboxControl {
        name: "Button",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "IconButton",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "ImageButton",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "PlainButton",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "Label",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "LinkLabel",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "TextField",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "AutoCompleteTextField",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "SearchField",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "TextArea",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "TextEditor",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "CheckBox",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "RadioButton",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "Toggle",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "ProgressBar",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "Slider",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "Stepper",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "ColorWell",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "DatePicker",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "DateTimePicker",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "TimePicker",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "DropDown",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "ComboBox",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "RadioGroup",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "SegmentedControl",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "ListBox",
        category: "Data",
    },
    ToolboxControl {
        name: "TreeView",
        category: "Data",
    },
    ToolboxControl {
        name: "DataGrid",
        category: "Data",
    },
    ToolboxControl {
        name: "TableView",
        category: "Data",
    },
    ToolboxControl {
        name: "Toolbar",
        category: "Menus & Toolbars",
    },
    ToolboxControl {
        name: "NavigationBar",
        category: "Menus & Toolbars",
    },
    ToolboxControl {
        name: "TabBar",
        category: "Menus & Toolbars",
    },
    ToolboxControl {
        name: "Card",
        category: "Containers",
    },
    ToolboxControl {
        name: "Expander",
        category: "Containers",
    },
    ToolboxControl {
        name: "FlowPanel",
        category: "Containers",
    },
    ToolboxControl {
        name: "GroupBox",
        category: "Containers",
    },
    ToolboxControl {
        name: "ScrollView",
        category: "Containers",
    },
    ToolboxControl {
        name: "SplitView",
        category: "Containers",
    },
    ToolboxControl {
        name: "StackPanel",
        category: "Containers",
    },
    ToolboxControl {
        name: "TableLayout",
        category: "Containers",
    },
    ToolboxControl {
        name: "Panel",
        category: "Containers",
    },
    ToolboxControl {
        name: "Divider",
        category: "Containers",
    },
    ToolboxControl {
        name: "Canvas",
        category: "Media",
    },
    ToolboxControl {
        name: "ImageView",
        category: "Media",
    },
];

pub fn populate_toolbox_tree(tree: &ui::TreeView, root: u32) {
    let mut current_category = "";
    let mut category_node = 0;
    for item in TOOLBOX_CONTROLS {
        if item.category != current_category {
            current_category = item.category;
            category_node = tree.add_child(root, current_category);
            tree.set_expanded(category_node, true);
        }
        tree.add_child(category_node, item.name);
    }
}
