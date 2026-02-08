use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use crate::drivers::input::keyboard::{self, Key};
use crate::drivers::input::mouse::{self, MouseEventType};
use crate::graphics::color::Color;
use crate::graphics::compositor::Compositor;
use crate::graphics::font;
use crate::graphics::rect::Rect;
use crate::graphics::renderer::Renderer;
use crate::ui::event::HitTest;
use crate::ui::menubar::{MenuBar, MenuAction};
use crate::ui::theme::Theme;
use crate::ui::window::Window;
use crate::sync::spinlock::Spinlock;

// ──────────────────────────────────────────────
// Global Desktop + event loop
// ──────────────────────────────────────────────

static DESKTOP: Spinlock<Option<Desktop>> = Spinlock::new(None);

// Window event types (shared with userland via stdlib)
pub const EVENT_KEY_DOWN: u32 = 1;
pub const EVENT_KEY_UP: u32 = 2;
pub const EVENT_RESIZE: u32 = 3;
pub const EVENT_MOUSE_DOWN: u32 = 4;
pub const EVENT_MOUSE_UP: u32 = 5;
pub const EVENT_MOUSE_MOVE: u32 = 6;

/// Initialize the global Desktop with framebuffer parameters.
pub fn init(width: u32, height: u32, fb_addr: u32, fb_pitch: u32) {
    let desktop = Desktop::new(width, height, fb_addr, fb_pitch);
    let mut guard = DESKTOP.lock();
    *guard = Some(desktop);
    crate::serial_println!("[OK] Desktop initialized ({}x{})", width, height);
}

/// Access the global Desktop within a closure. Returns None if not initialized.
pub fn with_desktop<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut Desktop) -> R,
{
    let mut guard = DESKTOP.lock();
    guard.as_mut().map(f)
}

/// Encode a Key enum into a u32 key code for userland events.
fn encode_key(key: &Key) -> u32 {
    match key {
        Key::Char(c) => *c as u32,
        Key::Enter => 0x100,
        Key::Backspace => 0x101,
        Key::Tab => 0x102,
        Key::Escape => 0x103,
        Key::Space => 0x104,
        Key::Up => 0x105,
        Key::Down => 0x106,
        Key::Left => 0x107,
        Key::Right => 0x108,
        Key::F1 => 0x110, Key::F2 => 0x111, Key::F3 => 0x112, Key::F4 => 0x113,
        Key::F5 => 0x114, Key::F6 => 0x115, Key::F7 => 0x116, Key::F8 => 0x117,
        Key::F9 => 0x118, Key::F10 => 0x119, Key::F11 => 0x11A, Key::F12 => 0x11B,
        Key::Delete => 0x120,
        Key::Home => 0x121,
        Key::End => 0x122,
        Key::PageUp => 0x123,
        Key::PageDown => 0x124,
        _ => 0,
    }
}

/// Encode modifiers into a bitmask for userland events.
fn encode_modifiers(m: &crate::drivers::input::keyboard::Modifiers) -> u32 {
    let mut flags = 0u32;
    if m.shift { flags |= 1; }
    if m.ctrl { flags |= 2; }
    if m.alt { flags |= 4; }
    if m.caps_lock { flags |= 8; }
    flags
}

/// Compositor task entry point — runs the event loop as a scheduled kernel task.
pub extern "C" fn desktop_task_entry() {
    // Enable interrupts — context_switch starts new threads with IF=0
    // to prevent races during the switch. We need IF=1 for hlt to wake.
    unsafe { core::arch::asm!("sti"); }

    // Flush stale mouse/keyboard events from PS/2 hardware initialization
    // (init acknowledgment bytes can be misinterpreted as button clicks)
    while crate::drivers::input::mouse::read_event().is_some() {}
    while crate::drivers::input::keyboard::read_event().is_some() {}

    crate::serial_println!("  Compositor task running");
    loop {
        let mut launch: Option<(String, String)> = None;
        {
            let mut guard = DESKTOP.lock();
            if let Some(desktop) = guard.as_mut() {
                // Process mouse events
                while let Some(event) = crate::drivers::input::mouse::read_event() {
                    let (cx, cy) = desktop.compositor.cursor_position();
                    let new_x = cx + event.dx;
                    let new_y = cy + event.dy;
                    desktop.compositor.move_cursor(new_x, new_y);

                    let (mx, my) = desktop.compositor.cursor_position();

                    match event.event_type {
                        MouseEventType::ButtonDown => {
                            desktop.handle_mouse_down(mx, my);
                        }
                        MouseEventType::ButtonUp => {
                            desktop.handle_mouse_up(mx, my);
                        }
                        MouseEventType::Move => {
                            desktop.handle_mouse_move(mx, my);
                        }
                    }
                }

                // Process keyboard events — forward to focused window's event queue
                while let Some(key_event) = keyboard::read_event() {
                    if key_event.pressed {
                        // Global shortcuts
                        if key_event.modifiers.ctrl && key_event.key == Key::Char('q') {
                            if let Some(id) = desktop.focused_window {
                                desktop.close_window(id);
                            }
                            continue;
                        }
                    }

                    // Forward to focused window's user event queue
                    if let Some(wid) = desktop.focused_window {
                        let event_type = if key_event.pressed { EVENT_KEY_DOWN } else { EVENT_KEY_UP };
                        let key_code = encode_key(&key_event.key);
                        let char_val = match key_event.key {
                            Key::Char(c) => c as u32,
                            Key::Space => ' ' as u32,
                            Key::Enter => '\n' as u32,
                            Key::Tab => '\t' as u32,
                            Key::Backspace => 0x08,
                            _ => 0,
                        };
                        let mods = encode_modifiers(&key_event.modifiers);
                        desktop.push_user_event(wid, [event_type, key_code, char_val, mods, 0]);
                    }
                }

                // Redraw menubar periodically for clock updates (~every 100 frames ≈ 1 sec at 100Hz)
                desktop.frame_count = desktop.frame_count.wrapping_add(1);
                if desktop.frame_count % 100 == 0 {
                    desktop.draw_menubar();
                }

                // Compose and present
                desktop.compositor.compose();

                // Extract pending launch (set by menu click)
                launch = desktop.pending_launch.take();
            }
        }

        // Process deferred launch OUTSIDE the Desktop lock to avoid deadlock
        if let Some((path, name)) = launch {
            crate::serial_println!("  Menu: launching '{}'...", path);
            match crate::task::loader::load_and_run(&path, &name) {
                Ok(tid) => {
                    crate::serial_println!("  Menu: launched '{}' (TID={})", name, tid);
                }
                Err(e) => {
                    crate::serial_println!("  Menu: failed to launch '{}': {}", path, e);
                }
            }
        }

        // Wait for next interrupt (timer tick) to avoid busy-spinning
        unsafe { core::arch::asm!("hlt"); }
    }
}

/// Desktop window manager
pub struct Desktop {
    pub compositor: Compositor,
    pub windows: Vec<Window>,
    pub menubar: MenuBar,
    /// ID of the focused window
    focused_window: Option<u32>,
    /// Next window ID
    next_window_id: u32,
    /// Current mouse interaction (drag or resize)
    interaction: Option<InteractionState>,
    /// Layer IDs for desktop, menubar
    desktop_layer: u32,
    menubar_layer: u32,
    /// Layer for the active dropdown menu (created on-demand, removed when closed)
    active_menu_layer: Option<u32>,
    /// Screen dimensions
    pub screen_width: u32,
    pub screen_height: u32,
    /// Per-window event queues for user-owned windows (window_id -> events)
    user_event_queues: BTreeMap<u32, VecDeque<[u32; 5]>>,
    /// Deferred app launch from menu click (path, name) — processed outside lock
    pending_launch: Option<(String, String)>,
    /// Frame counter for periodic redraws (clock)
    frame_count: u32,
}

enum InteractionState {
    Dragging {
        window_id: u32,
        offset_x: i32,
        offset_y: i32,
    },
    Resizing {
        window_id: u32,
        edge: HitTest,
        initial_x: i32,
        initial_y: i32,
        initial_w: u32,
        initial_h: u32,
        anchor_mx: i32,
        anchor_my: i32,
    },
}

impl Desktop {
    pub fn new(width: u32, height: u32, fb_addr: u32, fb_pitch: u32) -> Self {
        let mut compositor = Compositor::new(width, height, fb_addr, fb_pitch);

        // Create desktop background layer
        let desktop_layer = compositor.create_layer(0, 0, width, height);

        // Create menubar layer
        let menubar_layer = compositor.create_layer(0, 0, width, Theme::MENUBAR_HEIGHT);

        let mut desktop = Desktop {
            compositor,
            windows: Vec::new(),
            menubar: MenuBar::new(width),
            focused_window: None,
            next_window_id: 1,
            interaction: None,
            desktop_layer,
            menubar_layer,
            active_menu_layer: None,
            screen_width: width,
            screen_height: height,
            user_event_queues: BTreeMap::new(),
            pending_launch: None,
            frame_count: 0,
        };

        desktop.draw_desktop_background();
        desktop.draw_menubar();

        desktop
    }

    fn draw_desktop_background(&mut self) {
        if let Some(surface) = self.compositor.get_layer_surface(self.desktop_layer) {
            let mut renderer = Renderer::new(surface);

            // Dark gradient background
            renderer.fill_gradient_v(
                Rect::new(0, 0, self.screen_width, self.screen_height),
                Color::new(25, 25, 35),
                Color::new(15, 15, 25),
            );

            // Draw ".anyOS" text centered (use title size for the watermark)
            let title = ".anyOS";
            let wm_size = crate::ui::theme::Theme::FONT_SIZE_TITLE;
            let (tw, th) = font::measure_string_sized(title, wm_size);
            let tx = (self.screen_width as i32 - tw as i32) / 2;
            let ty = (self.screen_height as i32 - th as i32) / 2;
            drop(renderer);
            font::draw_string_sized(surface, tx, ty, title, Color::with_alpha(30, 255, 255, 255), wm_size);
            // Gradient uses opaque colors, mark surface for fast blit
            surface.opaque = true;
        }
    }

    fn draw_menubar(&mut self) {
        if let Some(surface) = self.compositor.get_layer_surface(self.menubar_layer) {
            self.menubar.render(surface);
            surface.opaque = true;
        }
    }

    /// Update the dropdown menu overlay layer.
    /// Creates/resizes/removes the layer as needed based on menu state.
    fn update_menu_overlay(&mut self) {
        if self.menubar.is_menu_open() {
            if let Some(bounds) = self.menubar.active_dropdown_bounds() {
                // Ensure overlay layer exists and matches dropdown bounds
                let need_create = match self.active_menu_layer {
                    Some(layer_id) => {
                        // Check if size changed (menu switched)
                        if let Some(layer) = self.compositor.get_layer(layer_id) {
                            layer.bounds() != bounds
                        } else {
                            true
                        }
                    }
                    None => true,
                };

                if need_create {
                    // Remove old layer if it exists
                    if let Some(old_id) = self.active_menu_layer.take() {
                        self.compositor.remove_layer(old_id);
                    }
                    // Create new layer sized to dropdown bounds
                    let layer_id = self.compositor.create_layer(
                        bounds.x, bounds.y, bounds.width, bounds.height,
                    );
                    self.active_menu_layer = Some(layer_id);
                }

                // Render menu into the layer
                if let Some(layer_id) = self.active_menu_layer {
                    // Only raise when newly created (avoid unnecessary damage from reordering)
                    if need_create {
                        self.compositor.raise_layer(layer_id);
                    }
                    if let Some(surface) = self.compositor.get_layer_surface(layer_id) {
                        surface.fill(Color::TRANSPARENT);
                        surface.opaque = false;
                        // Render dropdown at local coordinates (0,0) since layer is positioned at bounds.x/y
                        self.menubar.render_dropdown_at(surface, bounds.x, bounds.y);
                    }
                }
            }
        } else {
            // No menu open — remove overlay layer
            if let Some(layer_id) = self.active_menu_layer.take() {
                self.compositor.remove_layer(layer_id);
            }
        }
    }

    /// Execute a menu action
    fn execute_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::CloseWindow => {
                if let Some(id) = self.focused_window {
                    self.close_window(id);
                }
            }
            MenuAction::LaunchApp(path) => {
                let name = String::from(path.rsplit('/').next().unwrap_or("app"));
                self.pending_launch = Some((path, name));
            }
            MenuAction::About => {
                // TODO: show about dialog
                crate::serial_println!("  Menu: About .anyOS");
            }
            MenuAction::Restart => {
                crate::serial_println!("  Menu: Restart requested");
                // TODO: implement restart
            }
            MenuAction::Shutdown => {
                crate::serial_println!("  Menu: Shutdown requested");
                // TODO: implement shutdown
            }
        }
    }

    /// Raise all always-on-top windows above other windows.
    fn raise_always_on_top(&mut self) {
        for w in &self.windows {
            if w.always_on_top {
                self.compositor.raise_layer(w.layer_id);
            }
        }
    }

    /// Create a new window. flags: bit 0 = non-resizable, bit 1 = borderless, bit 2 = always-on-top.
    pub fn create_window(&mut self, title: &str, x: i32, y: i32, width: u32, height: u32, flags: u32) -> u32 {
        self.create_window_with_owner(title, x, y, width, height, flags, 0)
    }

    /// Create a new window owned by a specific thread.
    pub fn create_window_with_owner(&mut self, title: &str, x: i32, y: i32, width: u32, height: u32, flags: u32, owner_tid: u32) -> u32 {
        let id = self.next_window_id;
        self.next_window_id += 1;

        let mut window = Window::new(id, title, x, y, width, height);
        window.owner_tid = owner_tid;
        if flags & 1 != 0 {
            window.resizable = false;
        }
        if flags & 2 != 0 {
            window.borderless = true;
            window.resizable = false;
        }
        if flags & 4 != 0 {
            window.always_on_top = true;
        }

        // Create compositor layer
        let layer_id = self.compositor.create_layer(x, y, width, window.total_height());
        window.layer_id = layer_id;

        // Only change focus for normal (non-always-on-top) windows
        if !window.always_on_top {
            for w in &mut self.windows {
                if !w.always_on_top {
                    w.focused = false;
                    w.dirty = true;
                }
            }
            window.focused = true;
            self.focused_window = Some(id);
        }

        self.windows.push(window);

        // Render and update compositor
        self.render_window(id);

        // Update menubar with new focus (only for normal windows)
        if flags & 4 == 0 {
            if let Some(w) = self.windows.iter().find(|w| w.id == id) {
                self.menubar.set_app_name(&w.title);
                self.draw_menubar();
            }
        }

        // Re-raise always-on-top windows so they stay above
        self.raise_always_on_top();

        id
    }

    /// Close a window
    pub fn close_window(&mut self, id: u32) {
        if let Some(pos) = self.windows.iter().position(|w| w.id == id) {
            let window = self.windows.remove(pos);
            self.compositor.remove_layer(window.layer_id);
            self.user_event_queues.remove(&id);

            if self.focused_window == Some(id) {
                // Focus the topmost remaining non-always-on-top window
                if let Some(last) = self.windows.iter_mut().rev().find(|w| !w.always_on_top) {
                    last.focused = true;
                    last.dirty = true;
                    self.focused_window = Some(last.id);
                    let last_id = last.id;
                    self.render_window(last_id);
                    if let Some(w) = self.windows.iter().find(|w| w.id == last_id) {
                        self.menubar.set_app_name(&w.title);
                        self.draw_menubar();
                    }
                } else {
                    self.focused_window = None;
                    self.menubar.set_app_name("Finder");
                    self.draw_menubar();
                }
            }
        }
    }

    /// Close all windows owned by a specific thread (zombie cleanup on kill).
    pub fn close_windows_by_owner(&mut self, tid: u32) {
        // Collect window IDs to close (can't mutably iterate and close simultaneously)
        let ids: Vec<u32> = self.windows.iter()
            .filter(|w| w.owner_tid == tid)
            .map(|w| w.id)
            .collect();
        for id in ids {
            self.close_window(id);
        }
    }

    /// Public: Focus a window by ID (used by syscall)
    pub fn focus_window_by_id(&mut self, id: u32) {
        self.focus_window(id);
    }

    /// Focus a window (bring to front)
    fn focus_window(&mut self, id: u32) {
        if self.focused_window == Some(id) {
            return;
        }

        // Don't focus always-on-top windows (they don't participate in focus)
        if let Some(w) = self.windows.iter().find(|w| w.id == id) {
            if w.always_on_top {
                return;
            }
        }

        // Unfocus all non-always-on-top windows
        for w in &mut self.windows {
            if w.focused && !w.always_on_top {
                w.focused = false;
                w.dirty = true;
            }
        }

        // Focus the target
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
            w.focused = true;
            w.dirty = true;
            self.focused_window = Some(id);
            let layer_id = w.layer_id;
            self.compositor.raise_layer(layer_id);

            self.menubar.set_app_name(&w.title);
            self.draw_menubar();
        }

        // Re-render affected windows
        let ids: Vec<u32> = self.windows.iter().filter(|w| w.dirty).map(|w| w.id).collect();
        for wid in ids {
            self.render_window(wid);
        }

        // Keep always-on-top windows above
        self.raise_always_on_top();
    }

    /// Render a window's surface and update its compositor layer
    pub fn render_window(&mut self, id: u32) {
        // Phase 1: Find and render the window, extract metadata
        let win_idx = match self.windows.iter().position(|w| w.id == id) {
            Some(i) => i,
            None => return,
        };

        // Render the window (updates internal surface)
        self.windows[win_idx].render();

        let layer_id = self.windows[win_idx].layer_id;
        let win_w = self.windows[win_idx].width;
        let win_h = self.windows[win_idx].total_height();
        let is_borderless = self.windows[win_idx].borderless;

        // Phase 2: Copy rendered pixels to compositor layer
        // Using field-level borrow splitting (self.compositor vs self.windows)
        let needs_recreate = {
            if let Some(layer_surface) = self.compositor.get_layer_surface(layer_id) {
                if layer_surface.width == win_w && layer_surface.height == win_h {
                    // Sizes match, copy directly
                    let win_surface = self.windows[win_idx].surface();
                    layer_surface.pixels.copy_from_slice(&win_surface.pixels);
                    layer_surface.opaque = !is_borderless;
                    false
                } else {
                    true
                }
            } else {
                true
            }
        };

        if needs_recreate {
            let x = self.windows[win_idx].x;
            let y = self.windows[win_idx].y;
            self.compositor.remove_layer(layer_id);
            let new_layer_id = self.compositor.create_layer(x, y, win_w, win_h);
            self.windows[win_idx].layer_id = new_layer_id;

            if let Some(layer_surface) = self.compositor.get_layer_surface(new_layer_id) {
                let win_surface = self.windows[win_idx].surface();
                layer_surface.pixels.copy_from_slice(&win_surface.pixels);
                layer_surface.opaque = !is_borderless;
            }
        }
    }

    /// Get mutable access to a window's content surface
    pub fn window_content(&mut self, id: u32) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// Poll a user event from a window's event queue.
    /// Returns None if no events are pending.
    pub fn poll_user_event(&mut self, window_id: u32) -> Option<[u32; 5]> {
        self.user_event_queues.get_mut(&window_id)?.pop_front()
    }

    /// Push an event to a user window's event queue.
    pub fn push_user_event(&mut self, window_id: u32, event: [u32; 5]) {
        let queue = self.user_event_queues.entry(window_id).or_insert_with(VecDeque::new);
        if queue.len() < 256 { // Cap queue size
            queue.push_back(event);
        }
    }

    /// Process input events and update the desktop
    pub fn update(&mut self) {
        // Process mouse events
        while let Some(event) = mouse::read_event() {
            let (cx, cy) = self.compositor.cursor_position();
            let new_x = cx + event.dx;
            let new_y = cy + event.dy;
            self.compositor.move_cursor(new_x, new_y);

            let (mx, my) = self.compositor.cursor_position();

            match event.event_type {
                MouseEventType::ButtonDown => {
                    self.handle_mouse_down(mx, my);
                }
                MouseEventType::ButtonUp => {
                    self.handle_mouse_up(mx, my);
                }
                MouseEventType::Move => {
                    self.handle_mouse_move(mx, my);
                }
            }
        }

        // Process keyboard events — forward to focused window's event queue
        while let Some(key_event) = keyboard::read_event() {
            if key_event.pressed {
                if key_event.modifiers.ctrl && key_event.key == Key::Char('q') {
                    if let Some(id) = self.focused_window {
                        self.close_window(id);
                    }
                    continue;
                }
            }

            if let Some(wid) = self.focused_window {
                let event_type = if key_event.pressed { EVENT_KEY_DOWN } else { EVENT_KEY_UP };
                let key_code = encode_key(&key_event.key);
                let char_val = match key_event.key {
                    Key::Char(c) => c as u32,
                    Key::Space => ' ' as u32,
                    Key::Enter => '\n' as u32,
                    Key::Tab => '\t' as u32,
                    Key::Backspace => 0x08,
                    _ => 0,
                };
                let mods = encode_modifiers(&key_event.modifiers);
                self.push_user_event(wid, [event_type, key_code, char_val, mods, 0]);
            }
        }

        // Compose and present
        self.compositor.compose();
    }

    fn handle_mouse_down(&mut self, x: i32, y: i32) {
        // Check menubar FIRST (above everything)
        // If a menu is open, ALL clicks go through the menubar handler first
        if self.menubar.is_menu_open() || y < Theme::MENUBAR_HEIGHT as i32 {
            if let Some(action) = self.menubar.handle_click(x, y) {
                self.draw_menubar();
                self.update_menu_overlay();
                self.execute_menu_action(action);
                return;
            }
            // Menu state may have changed (opened/closed/switched)
            self.draw_menubar();
            self.update_menu_overlay();
            // If a menu is now open, consume the click
            if self.menubar.is_menu_open() || y < Theme::MENUBAR_HEIGHT as i32 {
                return;
            }
        }

        // Check if clicking on a window
        // Iterate windows back-to-front (top window first via compositor layer order)
        let mut clicked_window = None;
        let mut hit = HitTest::None;

        for window in self.windows.iter().rev() {
            let h = window.hit_test(x, y);
            if h != HitTest::None {
                clicked_window = Some(window.id);
                hit = h;
                break;
            }
        }

        if let Some(wid) = clicked_window {
            // Check if this is an always-on-top window (don't change focus)
            let is_always_on_top = self.windows.iter().find(|w| w.id == wid)
                .map(|w| w.always_on_top).unwrap_or(false);

            if !is_always_on_top {
                self.focus_window(wid);
            }

            match hit {
                HitTest::CloseButton => {
                    self.close_window(wid);
                }
                HitTest::MinimizeButton => {
                    // TODO: minimize
                }
                HitTest::MaximizeButton => {
                    if let Some(window) = self.windows.iter_mut().find(|w| w.id == wid) {
                        window.toggle_maximize(self.screen_width, self.screen_height);
                        window.dirty = true;
                        let layer_id = window.layer_id;
                        let wx = window.x;
                        let wy = window.y;
                        self.compositor.move_layer(layer_id, wx, wy);
                    }
                    self.render_window(wid);
                }
                HitTest::TitleBar => {
                    // Start drag
                    if let Some(window) = self.windows.iter().find(|w| w.id == wid) {
                        self.interaction = Some(InteractionState::Dragging {
                            window_id: wid,
                            offset_x: x - window.x,
                            offset_y: y - window.y,
                        });
                    }
                }
                HitTest::ResizeLeft | HitTest::ResizeRight | HitTest::ResizeBottom
                | HitTest::ResizeBottomLeft | HitTest::ResizeBottomRight => {
                    if let Some(window) = self.windows.iter().find(|w| w.id == wid) {
                        self.interaction = Some(InteractionState::Resizing {
                            window_id: wid,
                            edge: hit,
                            initial_x: window.x,
                            initial_y: window.y,
                            initial_w: window.width,
                            initial_h: window.height,
                            anchor_mx: x,
                            anchor_my: y,
                        });
                    }
                }
                HitTest::Client => {
                    // Send mouse events to the app (content-area local coords)
                    if let Some(window) = self.windows.iter().find(|w| w.id == wid) {
                        let lx = (x - window.x) as u32;
                        let ly = if window.borderless {
                            (y - window.y) as u32
                        } else {
                            (y - window.y - Theme::TITLEBAR_HEIGHT as i32) as u32
                        };
                        self.push_user_event(wid, [EVENT_MOUSE_DOWN, lx, ly, 1, 0]);
                    }
                }
                HitTest::None => {}
            }
        }
    }

    fn handle_mouse_up(&mut self, x: i32, y: i32) {
        // If we were resizing, send EVENT_RESIZE to the app
        if let Some(InteractionState::Resizing { window_id, .. }) = &self.interaction {
            let wid = *window_id;
            if let Some(window) = self.windows.iter().find(|w| w.id == wid) {
                let w = window.width;
                let h = window.height;
                self.push_user_event(wid, [EVENT_RESIZE, w, h, 0, 0]);
            }
        }

        // Forward mouse up to the topmost window containing the cursor
        for window in self.windows.iter().rev() {
            if window.bounds().contains(x, y) {
                let lx = (x - window.x) as u32;
                let ly = if window.borderless {
                    (y - window.y) as u32
                } else {
                    let raw = y - window.y - Theme::TITLEBAR_HEIGHT as i32;
                    if raw < 0 { break; } // click was in title bar, not content
                    raw as u32
                };
                let wid = window.id;
                self.push_user_event(wid, [EVENT_MOUSE_UP, lx, ly, 0, 0]);
                break;
            }
        }

        self.interaction = None;
    }

    fn handle_mouse_move(&mut self, x: i32, y: i32) {
        // If a dropdown menu is open, handle hover highlighting
        if self.menubar.is_menu_open() {
            self.menubar.handle_mouse_move(x, y);
            // Only update overlay (not full menubar) on hover changes
            self.update_menu_overlay();
            return; // Don't process window interactions while menu is open
        }

        use crate::ui::window::{MIN_WIDTH, MIN_HEIGHT};

        match &self.interaction {
            Some(InteractionState::Dragging { window_id, offset_x, offset_y }) => {
                let wid = *window_id;
                let new_x = x - offset_x;
                let new_y = y - offset_y;

                if let Some(window) = self.windows.iter_mut().find(|w| w.id == wid) {
                    window.x = new_x;
                    window.y = new_y;
                    let layer_id = window.layer_id;
                    self.compositor.move_layer(layer_id, new_x, new_y);
                }
            }
            Some(InteractionState::Resizing {
                window_id, edge, initial_x, initial_y, initial_w, initial_h, anchor_mx, anchor_my,
            }) => {
                let wid = *window_id;
                let edge = *edge;
                let ix = *initial_x;
                let iy = *initial_y;
                let iw = *initial_w;
                let ih = *initial_h;
                let amx = *anchor_mx;
                let amy = *anchor_my;
                let dx = x - amx;
                let dy = y - amy;

                let (mut new_x, new_y, mut new_w, mut new_h) = (ix, iy, iw, ih);

                match edge {
                    HitTest::ResizeRight => {
                        new_w = ((iw as i32 + dx).max(MIN_WIDTH as i32)) as u32;
                    }
                    HitTest::ResizeBottom => {
                        new_h = ((ih as i32 + dy).max(MIN_HEIGHT as i32)) as u32;
                    }
                    HitTest::ResizeLeft => {
                        let proposed_w = (iw as i32 - dx).max(MIN_WIDTH as i32) as u32;
                        new_x = ix + iw as i32 - proposed_w as i32; // pin right edge
                        new_w = proposed_w;
                    }
                    HitTest::ResizeBottomRight => {
                        new_w = ((iw as i32 + dx).max(MIN_WIDTH as i32)) as u32;
                        new_h = ((ih as i32 + dy).max(MIN_HEIGHT as i32)) as u32;
                    }
                    HitTest::ResizeBottomLeft => {
                        let proposed_w = (iw as i32 - dx).max(MIN_WIDTH as i32) as u32;
                        new_x = ix + iw as i32 - proposed_w as i32;
                        new_w = proposed_w;
                        new_h = ((ih as i32 + dy).max(MIN_HEIGHT as i32)) as u32;
                    }
                    _ => {}
                }

                if let Some(window) = self.windows.iter_mut().find(|w| w.id == wid) {
                    window.x = new_x;
                    window.y = new_y;
                    if window.width != new_w || window.height != new_h {
                        window.resize(new_w, new_h);
                    }
                    let layer_id = window.layer_id;
                    self.compositor.move_layer(layer_id, new_x, new_y);
                }
                self.render_window(wid);
            }
            None => {}
        }
    }

    /// Force a full redraw
    pub fn invalidate(&mut self) {
        self.draw_desktop_background();
        self.draw_menubar();
        self.update_menu_overlay();
        for i in 0..self.windows.len() {
            self.windows[i].dirty = true;
            let id = self.windows[i].id;
            self.render_window(id);
        }
        self.compositor.invalidate_all();
    }

    /// Enable hardware double-buffering via Bochs VGA.
    pub fn enable_hw_double_buffer(&mut self) {
        self.compositor.enable_hw_double_buffer();
    }

    /// Change display resolution. Resizes desktop, menubar, and notifies windows.
    pub fn change_resolution(&mut self, width: u32, height: u32) -> bool {
        if !self.compositor.change_resolution(width, height) {
            return false;
        }

        self.screen_width = width;
        self.screen_height = height;

        // Recreate desktop background layer at new size
        self.compositor.remove_layer(self.desktop_layer);
        self.desktop_layer = self.compositor.create_layer(0, 0, width, height);

        // Recreate menubar layer at new width
        self.compositor.remove_layer(self.menubar_layer);
        self.menubar_layer = self.compositor.create_layer(0, 0, width, Theme::MENUBAR_HEIGHT);
        self.menubar = MenuBar::new(width);

        // Redraw desktop and menubar
        self.draw_desktop_background();
        self.draw_menubar();

        // Notify all windows of resolution change
        let wids: Vec<u32> = self.windows.iter().map(|w| w.id).collect();
        for wid in &wids {
            if let Some(w) = self.windows.iter().find(|w| w.id == *wid) {
                self.push_user_event(*wid, [EVENT_RESIZE, w.width, w.height, 0, 0]);
            }
        }

        // Full invalidate
        self.invalidate();

        // Notify system event bus so userspace (dock, etc.) can adapt
        crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
            crate::ipc::event_bus::EVT_RESOLUTION_CHANGED, width, height, 0, 0,
        ));

        crate::serial_println!("[OK] Resolution changed to {}x{}", width, height);
        true
    }
}
