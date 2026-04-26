use alloc::string::String;
use alloc::vec::Vec;
use libanyui_client as ui;

#[derive(Clone, Copy, Debug)]
pub struct ToolboxControl {
    pub name: &'static str,
    pub category: &'static str,
}

#[derive(Clone, Debug)]
pub struct ToolboxNode {
    pub node: u32,
    pub control_name: String,
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

pub fn populate_toolbox_tree(tree: &ui::TreeView, root: u32) -> Vec<ToolboxNode> {
    let tc = ui::theme::colors();
    let mut current_category = "";
    let mut category_node = 0;
    let mut nodes = Vec::new();
    for item in TOOLBOX_CONTROLS {
        if item.category != current_category {
            current_category = item.category;
            category_node = tree.add_child(root, current_category);
            tree.set_node_text_color(category_node, tc.text_secondary);
            tree.set_expanded(category_node, true);
        }
        let node = tree.add_child(category_node, item.name);
        tree.set_node_text_color(node, tc.text);
        set_control_icon(tree, node, item.name, tc.text_secondary);
        if item.name != "Pointer" {
            nodes.push(ToolboxNode {
                node,
                control_name: String::from(item.name),
            });
        }
    }
    nodes
}

pub fn control_name_for_node(nodes: &[ToolboxNode], node: u32) -> Option<&str> {
    nodes
        .iter()
        .find(|entry| entry.node == node)
        .map(|entry| entry.control_name.as_str())
}

pub fn set_control_icon(tree: &ui::TreeView, node: u32, control_name: &str, color: u32) {
    if let Some(icon) = ui::Icon::system(
        icon_name_for_control(control_name),
        ui::IconType::Outline,
        color,
        16,
    ) {
        tree.set_node_icon(node, &icon.pixels, icon.width, icon.height);
    }
}

fn icon_name_for_control(control_name: &str) -> &'static str {
    match control_name {
        "Pointer" => "mouse-pointer-2",
        "Alert" => "triangle-alert",
        "Badge" => "badge",
        "StatusIndicator" => "circle-dot",
        "Spinner" => "loader",
        "Tag" => "tag",
        "Tooltip" => "message-circle-question",
        "Button" | "PlainButton" => "rectangle-horizontal",
        "IconButton" => "badge-icon",
        "ImageButton" => "image-up",
        "Label" => "type",
        "LinkLabel" => "link",
        "TextField" | "AutoCompleteTextField" | "SearchField" => "text-cursor-input",
        "TextArea" | "TextEditor" => "file-text",
        "CheckBox" => "square-check",
        "RadioButton" | "RadioGroup" => "circle-dot",
        "Toggle" => "toggle-right",
        "ProgressBar" => "chart-no-axes-column-increasing",
        "Slider" => "sliders-horizontal",
        "Stepper" => "plus-minus",
        "ColorWell" => "palette",
        "DatePicker" | "DateTimePicker" | "TimePicker" => "calendar-clock",
        "DropDown" | "ComboBox" => "list-collapse",
        "SegmentedControl" | "TabBar" => "panel-top",
        "ListBox" => "list",
        "TreeView" => "list-tree",
        "DataGrid" | "TableView" | "TableLayout" => "table-2",
        "Toolbar" => "panel-top-open",
        "NavigationBar" => "navigation",
        "Card" => "panel-top",
        "Expander" => "chevrons-up-down",
        "FlowPanel" => "layout-grid",
        "GroupBox" => "group",
        "ScrollView" => "scroll",
        "SplitView" => "columns-2",
        "StackPanel" => "rows-3",
        "Panel" => "panel-left",
        "Divider" => "separator-horizontal",
        "Canvas" => "brush",
        "ImageView" => "image",
        _ => "box",
    }
}
