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
        name: "Button",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "Label",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "TextField",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "TextArea",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "CheckBox",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "DropDown",
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
        name: "TabBar",
        category: "Containers",
    },
    ToolboxControl {
        name: "Toolbar",
        category: "Menus & Toolbars",
    },
    ToolboxControl {
        name: "SplitView",
        category: "Containers",
    },
    ToolboxControl {
        name: "Panel",
        category: "Containers",
    },
    ToolboxControl {
        name: "ImageView",
        category: "Media",
    },
    ToolboxControl {
        name: "ProgressBar",
        category: "Common Controls",
    },
    ToolboxControl {
        name: "Slider",
        category: "Common Controls",
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
