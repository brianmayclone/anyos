use crate::{events, lib};

/// Menu item flags.
pub const MENU_FLAG_DISABLED: u32 = 0x01;
pub const MENU_FLAG_SEPARATOR: u32 = 0x02;
pub const MENU_FLAG_CHECKED: u32 = 0x04;

const MENU_MAGIC: u32 = 0x4D454E55; // "MENU"

/// Event passed to the on_menu_item callback.
pub struct MenuItemEvent {
    pub item_id: u32,
}

/// A menu bar attached to a window.
///
/// Built using `MenuBarBuilder`, then applied to a window via `set()`.
/// Menu item clicks are dispatched via `on_item()`.
pub struct MenuBar {
    win_id: u32,
}

impl MenuBar {
    /// Attach a menu bar to a window using pre-built binary data.
    pub fn set(win_id: u32, data: &[u8]) -> Self {
        (lib().set_menu_fn)(win_id, data.as_ptr(), data.len() as u32);
        Self { win_id }
    }

    /// Register a callback for menu item clicks on this window.
    pub fn on_item(&self, mut f: impl FnMut(&MenuItemEvent) + 'static) {
        let (thunk, ud) = events::register(move |item_id, _| {
            f(&MenuItemEvent { item_id });
        });
        (lib().on_menu_item_fn)(self.win_id, thunk, ud);
    }

    /// Update a menu item's flags (enable/disable/check).
    pub fn update_item(&self, item_id: u32, new_flags: u32) {
        (lib().update_menu_item_fn)(self.win_id, item_id, new_flags);
    }
}

/// Builder for constructing a binary menu bar definition.
///
/// ```rust,ignore
/// let mut builder = MenuBarBuilder::new();
/// let data = builder
///     .menu("File")
///         .item(1, "New", 0)
///         .item(2, "Open...", 0)
///         .separator()
///         .item(5, "Quit", 0)
///     .end_menu()
///     .menu("Edit")
///         .item(10, "Cut", 0)
///         .item(11, "Copy", 0)
///         .item(12, "Paste", 0)
///     .end_menu()
///     .build();
/// let menu = MenuBar::set(win.id(), data);
/// menu.on_item(|e| match e.item_id { ... });
/// ```
pub struct MenuBarBuilder {
    buf: [u8; 4096],
    pos: usize,
    num_menus: usize,
    num_menus_offset: usize,
}

pub struct MenuBuilder {
    inner: MenuBarBuilder,
    num_items: usize,
    num_items_offset: usize,
}

impl MenuBarBuilder {
    pub fn new() -> Self {
        let mut b = MenuBarBuilder {
            buf: [0u8; 4096],
            pos: 0,
            num_menus: 0,
            num_menus_offset: 0,
        };
        b.write_u32(MENU_MAGIC);
        b.num_menus_offset = b.pos;
        b.write_u32(0);
        b
    }

    pub fn menu(mut self, title: &str) -> MenuBuilder {
        let bytes = title.as_bytes();
        let len = bytes.len().min(64);
        self.write_u32(len as u32);
        self.write_bytes(&bytes[..len]);
        self.align4();
        let num_items_offset = self.pos;
        self.write_u32(0);
        self.num_menus += 1;
        MenuBuilder {
            inner: self,
            num_items: 0,
            num_items_offset,
        }
    }

    pub fn build(&mut self) -> &[u8] {
        let nm = self.num_menus as u32;
        self.buf[self.num_menus_offset..self.num_menus_offset + 4]
            .copy_from_slice(&nm.to_le_bytes());
        &self.buf[..self.pos]
    }

    fn write_u32(&mut self, val: u32) {
        if self.pos + 4 <= self.buf.len() {
            self.buf[self.pos..self.pos + 4].copy_from_slice(&val.to_le_bytes());
            self.pos += 4;
        }
    }

    fn write_bytes(&mut self, data: &[u8]) {
        let end = (self.pos + data.len()).min(self.buf.len());
        let count = end - self.pos;
        self.buf[self.pos..self.pos + count].copy_from_slice(&data[..count]);
        self.pos += count;
    }

    fn align4(&mut self) {
        while self.pos % 4 != 0 && self.pos < self.buf.len() {
            self.buf[self.pos] = 0;
            self.pos += 1;
        }
    }
}

impl MenuBuilder {
    pub fn item(mut self, item_id: u32, label: &str, flags: u32) -> Self {
        self.inner.write_u32(item_id);
        self.inner.write_u32(flags);
        let bytes = label.as_bytes();
        let len = bytes.len().min(64);
        self.inner.write_u32(len as u32);
        self.inner.write_bytes(&bytes[..len]);
        self.inner.align4();
        self.num_items += 1;
        self
    }

    pub fn separator(self) -> Self {
        self.item(0, "", MENU_FLAG_SEPARATOR)
    }

    pub fn end_menu(mut self) -> MenuBarBuilder {
        let ni = self.num_items as u32;
        self.inner.buf[self.num_items_offset..self.num_items_offset + 4]
            .copy_from_slice(&ni.to_le_bytes());
        self.inner
    }
}
