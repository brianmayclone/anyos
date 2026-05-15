// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Background network worker thread for the Surf browser.
//!
//! Moves all blocking HTTP fetches (navigation, CSS, images) off the
//! UI thread onto a dedicated worker.  Communication uses static shared
//! state guarded by `AtomicBool` spinlocks since `Thread::spawn` only
//! accepts `fn()` (not closures).

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use libanyui_client as ui_lib;

use crate::http::{self, ConnPool, CookieJar, FetchError, Url};

pub(crate) struct DecodedCss {
    pub sheet: libwebview::css::Stylesheet,
}

pub(crate) struct DecodedRaster {
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub suspicious_black_ppm: Option<u32>,
}

macro_rules! surf_net_log {
    ($($arg:tt)*) => {
        anyos_std::println!(
            "[surf-net {:>8}ms] {}",
            anyos_std::sys::uptime_ms(),
            alloc::format!($($arg)*)
        )
    };
}

// ═══════════════════════════════════════════════════════════
// Request / result types
// ═══════════════════════════════════════════════════════════

/// A fetch request submitted by the UI thread.
pub(crate) enum FetchRequest {
    /// Full page navigation (GET): fetch HTML, return body + headers + cookies.
    Navigate {
        tab_index: usize,
        url: Url,
        cookies: CookieJar,
        generation: u32,
    },
    /// Full page navigation (POST): fetch HTML with form body.
    NavigatePost {
        tab_index: usize,
        url: Url,
        body: String,
        cookies: CookieJar,
        generation: u32,
    },
    /// External CSS stylesheet fetch.
    Css {
        tab_index: usize,
        href: String,
        url: Url,
        generation: u32,
    },
    /// External image fetch.
    Image {
        tab_index: usize,
        src: String,
        url: Url,
        target_width: Option<u32>,
        target_height: Option<u32>,
        priority: ImagePriority,
        from_deferred: bool,
        generation: u32,
    },
    /// Web font fetch (@font-face src).
    Font {
        tab_index: usize,
        family: String,
        weight: u32,
        italic: bool,
        url: Url,
        display: libwebview::css::FontDisplay,
        generation: u32,
    },
    /// External `<script src="...">` fetch.
    Script {
        tab_index: usize,
        /// Slot index in the tab's `pending_scripts` array (preserves document order).
        slot: usize,
        src: String,
        url: Url,
        generation: u32,
    },
    /// ES module dependency fetch discovered from a script import.
    ModuleScript {
        tab_index: usize,
        specifier: String,
        url: Url,
        generation: u32,
    },
}

struct QueuedFetchRequest {
    req: FetchRequest,
    request_id: u32,
    submitted_ms: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImagePriority {
    Viewport,
    Deferred,
}

/// A completed fetch result returned by the worker thread.
pub(crate) enum FetchResult {
    /// Navigation completed successfully.
    NavDone {
        tab_index: usize,
        response: http::Response,
        url: Url,
        cookies: CookieJar,
        generation: u32,
    },
    /// Navigation failed.
    NavError {
        tab_index: usize,
        error_msg: &'static str,
        generation: u32,
    },
    /// CSS fetch completed successfully.
    CssDone {
        tab_index: usize,
        href: String,
        url: Url,
        body: Vec<u8>,
        headers: String,
        parsed: Option<DecodedCss>,
        timing: Option<http::RequestTiming>,
        from_cache: bool,
        generation: u32,
    },
    /// Image fetch completed successfully.
    ImageDone {
        tab_index: usize,
        src: String,
        url: Url,
        body: Vec<u8>,
        encoded_len: usize,
        headers: String,
        decoded_raster: Option<DecodedRaster>,
        priority: ImagePriority,
        from_deferred: bool,
        timing: Option<http::RequestTiming>,
        from_cache: bool,
        generation: u32,
    },
    /// Web font fetch completed successfully.
    FontDone {
        tab_index: usize,
        family: String,
        weight: u32,
        italic: bool,
        url: Url,
        body: Vec<u8>,
        display: libwebview::css::FontDisplay,
        timing: Option<http::RequestTiming>,
        from_cache: bool,
        generation: u32,
    },
    /// External script fetch completed successfully.
    ScriptDone {
        tab_index: usize,
        /// Slot index — matches the request's `slot` so the UI thread can
        /// place the fetched text at the correct position in document order.
        slot: usize,
        url: Url,
        body: Vec<u8>,
        headers: String,
        timing: Option<http::RequestTiming>,
        from_cache: bool,
        generation: u32,
    },
    /// ES module dependency fetch completed.
    ModuleScriptDone {
        tab_index: usize,
        specifier: String,
        url: Url,
        body: Vec<u8>,
        headers: String,
        timing: Option<http::RequestTiming>,
        from_cache: bool,
        generation: u32,
    },
}

impl FetchResult {
    pub(crate) fn tab_index(&self) -> usize {
        match self {
            FetchResult::NavDone { tab_index, .. }
            | FetchResult::NavError { tab_index, .. }
            | FetchResult::CssDone { tab_index, .. }
            | FetchResult::ImageDone { tab_index, .. }
            | FetchResult::FontDone { tab_index, .. }
            | FetchResult::ScriptDone { tab_index, .. }
            | FetchResult::ModuleScriptDone { tab_index, .. } => *tab_index,
        }
    }
}

// ═══════════════════════════════════════════════════════════
// Shared state + spinlock
// ═══════════════════════════════════════════════════════════

static REQUEST_LOCK_CRITICAL: AtomicBool = AtomicBool::new(false);
static REQUEST_LOCK_TLS: AtomicBool = AtomicBool::new(false);
static REQUEST_LOCK_SCRIPT: AtomicBool = AtomicBool::new(false);
static REQUEST_LOCK_FONT: AtomicBool = AtomicBool::new(false);
static REQUEST_LOCK_VISIBLE: AtomicBool = AtomicBool::new(false);
static REQUEST_LOCK_BACKGROUND: AtomicBool = AtomicBool::new(false);
static RESULT_LOCK: AtomicBool = AtomicBool::new(false);
static CACHE_LOCK: AtomicBool = AtomicBool::new(false);
static HOST_LIMIT_LOCK: AtomicBool = AtomicBool::new(false);
const RESULT_MAILBOX_COUNT: usize = 16;
const MAX_CONNECTIONS_PER_HOST: u32 = 8;
const CRITICAL_WORKER_LANES: u32 = 4;
const TLS_WORKER_LANES: u32 = 8;
const SCRIPT_WORKER_LANES: u32 = 4;
const FONT_WORKER_LANES: u32 = 4;
const VISIBLE_WORKER_LANES: u32 = 8;
const BACKGROUND_WORKER_LANES: u32 = 8;

static mut REQUEST_QUEUE_CRITICAL: Option<Vec<QueuedFetchRequest>> = None;
static mut REQUEST_QUEUE_TLS: Option<Vec<QueuedFetchRequest>> = None;
static mut REQUEST_QUEUE_SCRIPT: Option<Vec<QueuedFetchRequest>> = None;
static mut REQUEST_QUEUE_FONT: Option<Vec<QueuedFetchRequest>> = None;
static mut REQUEST_QUEUE_VISIBLE: Option<Vec<QueuedFetchRequest>> = None;
static mut REQUEST_QUEUE_BACKGROUND: Option<Vec<QueuedFetchRequest>> = None;
static mut RESULT_MAILBOXES: Option<Vec<Vec<FetchResult>>> = None;
static mut SUBRESOURCE_CACHE: Option<SubResourceCache> = None;
static mut HOST_ACTIVE_COUNTS: Option<Vec<HostActiveCount>> = None;

struct HostActiveCount {
    host: String,
    count: u32,
}

/// Generation counter — incremented on each Navigate request.
/// Worker skips CSS/Image requests with a stale generation.
static GENERATION: AtomicU32 = AtomicU32::new(0);

/// Active worker counts per class.
static WORKER_ACTIVE_CRITICAL: AtomicU32 = AtomicU32::new(0);
static WORKER_ACTIVE_TLS: AtomicU32 = AtomicU32::new(0);
static WORKER_ACTIVE_SCRIPT: AtomicU32 = AtomicU32::new(0);
static WORKER_ACTIVE_FONT: AtomicU32 = AtomicU32::new(0);
static WORKER_ACTIVE_VISIBLE: AtomicU32 = AtomicU32::new(0);
static WORKER_ACTIVE_BACKGROUND: AtomicU32 = AtomicU32::new(0);
/// Requests currently being processed after they have been dequeued.
static REQUESTS_IN_FLIGHT: AtomicU32 = AtomicU32::new(0);
static RESULT_NOTIFY_PENDING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkerClass {
    Critical,
    Tls,
    Script,
    Font,
    Visible,
    Background,
}

/// Acquire a spinlock. Spins with hint to avoid wasting CPU.
fn acquire(lock: &AtomicBool) {
    loop {
        if lock
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            return;
        }
        core::hint::spin_loop();
    }
}

/// Release a spinlock.
fn release(lock: &AtomicBool) {
    lock.store(false, Ordering::Release);
}

fn worker_label(class: WorkerClass) -> &'static str {
    match class {
        WorkerClass::Critical => "critical",
        WorkerClass::Tls => "tls",
        WorkerClass::Script => "script",
        WorkerClass::Font => "font",
        WorkerClass::Visible => "visible",
        WorkerClass::Background => "background",
    }
}

fn request_lock(class: WorkerClass) -> &'static AtomicBool {
    match class {
        WorkerClass::Critical => &REQUEST_LOCK_CRITICAL,
        WorkerClass::Tls => &REQUEST_LOCK_TLS,
        WorkerClass::Script => &REQUEST_LOCK_SCRIPT,
        WorkerClass::Font => &REQUEST_LOCK_FONT,
        WorkerClass::Visible => &REQUEST_LOCK_VISIBLE,
        WorkerClass::Background => &REQUEST_LOCK_BACKGROUND,
    }
}

fn worker_active_count(class: WorkerClass) -> &'static AtomicU32 {
    match class {
        WorkerClass::Critical => &WORKER_ACTIVE_CRITICAL,
        WorkerClass::Tls => &WORKER_ACTIVE_TLS,
        WorkerClass::Script => &WORKER_ACTIVE_SCRIPT,
        WorkerClass::Font => &WORKER_ACTIVE_FONT,
        WorkerClass::Visible => &WORKER_ACTIVE_VISIBLE,
        WorkerClass::Background => &WORKER_ACTIVE_BACKGROUND,
    }
}

fn worker_lane_target(class: WorkerClass) -> u32 {
    match class {
        WorkerClass::Critical => CRITICAL_WORKER_LANES,
        WorkerClass::Tls => TLS_WORKER_LANES,
        WorkerClass::Script => SCRIPT_WORKER_LANES,
        WorkerClass::Font => FONT_WORKER_LANES,
        WorkerClass::Visible => VISIBLE_WORKER_LANES,
        WorkerClass::Background => BACKGROUND_WORKER_LANES,
    }
}

unsafe fn request_queue_mut(class: WorkerClass) -> Option<&'static mut Vec<QueuedFetchRequest>> {
    match class {
        WorkerClass::Critical => REQUEST_QUEUE_CRITICAL.as_mut(),
        WorkerClass::Tls => REQUEST_QUEUE_TLS.as_mut(),
        WorkerClass::Script => REQUEST_QUEUE_SCRIPT.as_mut(),
        WorkerClass::Font => REQUEST_QUEUE_FONT.as_mut(),
        WorkerClass::Visible => REQUEST_QUEUE_VISIBLE.as_mut(),
        WorkerClass::Background => REQUEST_QUEUE_BACKGROUND.as_mut(),
    }
}

unsafe fn request_queue_ref(class: WorkerClass) -> Option<&'static Vec<QueuedFetchRequest>> {
    match class {
        WorkerClass::Critical => REQUEST_QUEUE_CRITICAL.as_ref(),
        WorkerClass::Tls => REQUEST_QUEUE_TLS.as_ref(),
        WorkerClass::Script => REQUEST_QUEUE_SCRIPT.as_ref(),
        WorkerClass::Font => REQUEST_QUEUE_FONT.as_ref(),
        WorkerClass::Visible => REQUEST_QUEUE_VISIBLE.as_ref(),
        WorkerClass::Background => REQUEST_QUEUE_BACKGROUND.as_ref(),
    }
}

fn queued_request_count(class: WorkerClass) -> u32 {
    let lock = request_lock(class);
    acquire(lock);
    let count = unsafe {
        request_queue_ref(class)
            .map(|q| q.len().min(u32::MAX as usize) as u32)
            .unwrap_or(0)
    };
    release(lock);
    count
}

fn request_worker_class(req: &FetchRequest) -> WorkerClass {
    match req {
        FetchRequest::Navigate { .. }
        | FetchRequest::NavigatePost { .. }
        | FetchRequest::Css { .. } => WorkerClass::Critical,
        FetchRequest::Script { .. } | FetchRequest::ModuleScript { .. } => WorkerClass::Script,
        FetchRequest::Font { .. } => WorkerClass::Font,
        FetchRequest::Image { priority, .. } => match priority {
            ImagePriority::Viewport => WorkerClass::Visible,
            ImagePriority::Deferred => WorkerClass::Background,
        },
    }
}

fn ensure_worker(class: WorkerClass) {
    let target = worker_lane_target(class);
    let active = worker_active_count(class);
    let pending = queued_request_count(class);
    let wanted = active
        .load(Ordering::Relaxed)
        .saturating_add(pending)
        .min(target);
    if wanted == 0 {
        return;
    }

    loop {
        let reserved = active.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            if current < wanted {
                Some(current + 1)
            } else {
                None
            }
        });
        if reserved.is_err() {
            return;
        }

        let (entry, thread_name): (fn(), &str) = match class {
            WorkerClass::Critical => (worker_entry_critical, "surf-net-crit"),
            WorkerClass::Tls => (worker_entry_tls, "surf-net-tls"),
            WorkerClass::Script => (worker_entry_script, "surf-net-script"),
            WorkerClass::Font => (worker_entry_font, "surf-net-font"),
            WorkerClass::Visible => (worker_entry_visible, "surf-net-vis"),
            WorkerClass::Background => (worker_entry_background, "surf-net-bg"),
        };
        match anyos_std::process::Thread::spawn_with_stack(entry, 256 * 1024, thread_name) {
            Ok(handle) => {
                core::mem::forget(handle);
                surf_net_log!(
                    "{} worker thread started ({}/{})",
                    worker_label(class),
                    active.load(Ordering::Relaxed),
                    target
                );
            }
            Err(_) => {
                active.fetch_sub(1, Ordering::SeqCst);
                surf_net_log!(
                    "ERROR: failed to spawn {} worker thread",
                    worker_label(class)
                );
                return;
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════
// Public API (called from UI thread)
// ═══════════════════════════════════════════════════════════

/// Initialize the shared queues. Must be called once from the UI thread
/// before any requests are submitted.
pub(crate) fn init() {
    unsafe {
        REQUEST_QUEUE_CRITICAL = Some(Vec::new());
        REQUEST_QUEUE_TLS = Some(Vec::new());
        REQUEST_QUEUE_SCRIPT = Some(Vec::new());
        REQUEST_QUEUE_FONT = Some(Vec::new());
        REQUEST_QUEUE_VISIBLE = Some(Vec::new());
        REQUEST_QUEUE_BACKGROUND = Some(Vec::new());
        RESULT_MAILBOXES = Some((0..RESULT_MAILBOX_COUNT).map(|_| Vec::new()).collect());
        SUBRESOURCE_CACHE = Some(SubResourceCache::new());
        HOST_ACTIVE_COUNTS = Some(Vec::new());
    }
}

extern "C" fn result_ready_cb(_userdata: u64) {
    RESULT_NOTIFY_PENDING.store(false, Ordering::Release);
    crate::handle_net_worker_results_ready();
}

fn notify_result_ready() {
    if RESULT_NOTIFY_PENDING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
    {
        ui_lib::marshal_dispatch(result_ready_cb, 0);
    }
}

/// Submit a request to the worker queue.
pub(crate) fn submit(req: FetchRequest) {
    let submitted_ms = anyos_std::sys::uptime_ms();
    let request_id = record_started(&req);
    let class = request_worker_class(&req);
    let lock = request_lock(class);
    acquire(lock);
    unsafe {
        if let Some(q) = request_queue_mut(class) {
            q.push(QueuedFetchRequest {
                req,
                request_id,
                submitted_ms,
            });
        }
    }
    release(lock);
    ensure_worker(class);
}

/// Record a started request in the DevTools network panel (UI thread).
fn record_started(req: &FetchRequest) -> u32 {
    match req {
        FetchRequest::Navigate { url, .. } => {
            crate::devtools::record_request_started("GET", "html", url)
        }
        FetchRequest::NavigatePost { url, .. } => {
            crate::devtools::record_request_started("POST", "html", url)
        }
        FetchRequest::Css { url, .. } => crate::devtools::record_request_started("GET", "css", url),
        FetchRequest::Image { url, .. } => {
            crate::devtools::record_request_started("GET", "img", url)
        }
        FetchRequest::Font { url, .. } => {
            crate::devtools::record_request_started("GET", "font", url)
        }
        FetchRequest::Script { url, .. } | FetchRequest::ModuleScript { url, .. } => {
            crate::devtools::record_request_started("GET", "js", url)
        }
    }
}

/// Drain completed results for one tab from its mailbox.
pub(crate) fn drain_results_for_tab(tab_index: usize) -> Vec<FetchResult> {
    acquire(&RESULT_LOCK);
    let results = unsafe {
        if let Some(mailboxes) = RESULT_MAILBOXES.as_mut() {
            if let Some(mailbox) = mailboxes.get_mut(tab_index) {
                if mailbox.is_empty() {
                    Vec::new()
                } else {
                    surf_net_log!("mailbox poll: tab={} pending={}", tab_index, mailbox.len());
                    core::mem::replace(mailbox, Vec::new())
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    };
    release(&RESULT_LOCK);
    results
}

#[cfg(feature = "debug_surf")]
pub(crate) fn mailbox_pending_counts() -> Vec<usize> {
    acquire(&RESULT_LOCK);
    let counts = unsafe {
        RESULT_MAILBOXES
            .as_ref()
            .map(|mailboxes| mailboxes.iter().map(|mailbox| mailbox.len()).collect())
            .unwrap_or_default()
    };
    release(&RESULT_LOCK);
    counts
}

pub(crate) fn result_mailboxes_pending() -> bool {
    acquire(&RESULT_LOCK);
    let pending = unsafe {
        RESULT_MAILBOXES
            .as_ref()
            .is_some_and(|mailboxes| mailboxes.iter().any(|mailbox| !mailbox.is_empty()))
    };
    release(&RESULT_LOCK);
    pending
}

fn result_mailbox_len_for_tab(tab_index: usize) -> usize {
    acquire(&RESULT_LOCK);
    let len = unsafe {
        RESULT_MAILBOXES
            .as_ref()
            .and_then(|mailboxes| mailboxes.get(tab_index))
            .map(|mailbox| mailbox.len())
            .unwrap_or(0)
    };
    release(&RESULT_LOCK);
    len
}

/// Requeue results to the front of a tab mailbox so they are seen on the next UI poll.
pub(crate) fn prepend_results_for_tab(tab_index: usize, mut results: Vec<FetchResult>) {
    let requeued = results.len();
    if requeued == 0 {
        return;
    }
    acquire(&RESULT_LOCK);
    unsafe {
        if let Some(mailboxes) = RESULT_MAILBOXES.as_mut() {
            if let Some(mailbox) = mailboxes.get_mut(tab_index) {
                let pending = core::mem::replace(mailbox, Vec::new());
                results.extend(pending);
                *mailbox = results;
                surf_net_log!(
                    "mailbox requeue: tab={} requeued={} pending_now={}",
                    tab_index,
                    requeued,
                    mailbox.len()
                );
            }
        }
    }
    release(&RESULT_LOCK);
    notify_result_ready();
}

/// Bump the generation counter and clear any pending CSS/Image requests
/// from previous pages. Called when a new Navigate begins.
pub(crate) fn new_generation() -> u32 {
    let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    // Clear pending resource requests for old generations.
    for class in [
        WorkerClass::Critical,
        WorkerClass::Tls,
        WorkerClass::Script,
        WorkerClass::Font,
        WorkerClass::Visible,
        WorkerClass::Background,
    ] {
        let lock = request_lock(class);
        acquire(lock);
        unsafe {
            if let Some(q) = request_queue_mut(class) {
                q.retain(|r| match &r.req {
                    FetchRequest::Navigate { .. } | FetchRequest::NavigatePost { .. } => true,
                    FetchRequest::Css { generation, .. }
                    | FetchRequest::Image { generation, .. }
                    | FetchRequest::Font { generation, .. }
                    | FetchRequest::Script { generation, .. }
                    | FetchRequest::ModuleScript { generation, .. } => *generation == gen,
                });
            }
        }
        release(lock);
    }
    // Also clear stale results.
    acquire(&RESULT_LOCK);
    unsafe {
        if let Some(mailboxes) = RESULT_MAILBOXES.as_mut() {
            for mailbox in mailboxes.iter_mut() {
                mailbox.retain(|r| match r {
                    FetchResult::NavDone { .. } | FetchResult::NavError { .. } => true,
                    FetchResult::CssDone { generation, .. }
                    | FetchResult::ImageDone { generation, .. }
                    | FetchResult::FontDone { generation, .. }
                    | FetchResult::ScriptDone { generation, .. }
                    | FetchResult::ModuleScriptDone { generation, .. } => *generation == gen,
                });
            }
        }
    }
    release(&RESULT_LOCK);
    gen
}

/// Get the current generation counter.
pub(crate) fn current_generation() -> u32 {
    GENERATION.load(Ordering::Relaxed)
}

pub(crate) fn handle_closed_tab(closed_idx: usize) {
    for class in [
        WorkerClass::Critical,
        WorkerClass::Tls,
        WorkerClass::Script,
        WorkerClass::Font,
        WorkerClass::Visible,
        WorkerClass::Background,
    ] {
        let lock = request_lock(class);
        acquire(lock);
        unsafe {
            if let Some(q) = request_queue_mut(class) {
                let mut old = core::mem::replace(q, Vec::new());
                for mut queued in old.drain(..) {
                    let tab_index = request_tab_index_mut(&mut queued.req);
                    if *tab_index == closed_idx {
                        continue;
                    }
                    if *tab_index > closed_idx {
                        *tab_index -= 1;
                    }
                    q.push(queued);
                }
            }
        }
        release(lock);
    }

    acquire(&RESULT_LOCK);
    unsafe {
        if let Some(mailboxes) = RESULT_MAILBOXES.as_mut() {
            if closed_idx < mailboxes.len() {
                mailboxes.remove(closed_idx);
                mailboxes.push(Vec::new());
            }
        }
    }
    release(&RESULT_LOCK);
    notify_result_ready();
}

/// Best-effort signal for the UI thread that the network worker may still
/// produce results soon.
pub(crate) fn has_pending_activity() -> bool {
    if REQUESTS_IN_FLIGHT.load(Ordering::Relaxed) > 0 {
        return true;
    }

    for class in [
        WorkerClass::Critical,
        WorkerClass::Tls,
        WorkerClass::Script,
        WorkerClass::Font,
        WorkerClass::Visible,
        WorkerClass::Background,
    ] {
        let lock = request_lock(class);
        acquire(lock);
        let request_non_empty = unsafe { request_queue_ref(class).is_some_and(|q| !q.is_empty()) };
        release(lock);
        if request_non_empty {
            return true;
        }
    }

    acquire(&RESULT_LOCK);
    let result_non_empty = unsafe {
        RESULT_MAILBOXES
            .as_ref()
            .is_some_and(|mailboxes| mailboxes.iter().any(|mailbox| !mailbox.is_empty()))
    };
    release(&RESULT_LOCK);
    result_non_empty
}

// ═══════════════════════════════════════════════════════════
// Sub-resource cache
// ═══════════════════════════════════════════════════════════

/// Maximum number of cached sub-resource responses.
const MAX_CACHE_ENTRIES: usize = 1024;
const MAX_CACHE_BYTES: usize = 512 * 1024 * 1024;
const MAX_CACHEABLE_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHEABLE_IMAGE_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_DISK_CACHEABLE_BODY_BYTES: usize = 32 * 1024 * 1024;
const DISK_CACHE_MIN_FREE_BYTES: u64 = 512 * 1024 * 1024;
const DISK_CACHE_DEFAULT_MAX_AGE_SECS: u32 = 60 * 60;
/// A cached HTTP response for a sub-resource.
struct CacheEntry {
    /// Cache key: fully-qualified URL string.
    url_key: String,
    /// Raw response body.
    body: Vec<u8>,
    /// Response headers.
    headers: String,
    /// Unix timestamp when the response entered the cache.
    stored_secs: u32,
}

/// Shared sub-resource cache for CSS, scripts, images, and fonts.
struct SubResourceCache {
    entries: Vec<CacheEntry>,
    total_bytes: usize,
}

impl SubResourceCache {
    fn new() -> Self {
        SubResourceCache {
            entries: Vec::new(),
            total_bytes: 0,
        }
    }

    fn get(&self, url_key: &str) -> Option<(Vec<u8>, String)> {
        self.entries
            .iter()
            .find(|e| {
                e.url_key == url_key && cache_stored_response_is_fresh(e.stored_secs, &e.headers)
            })
            .map(|e| (e.body.clone(), e.headers.clone()))
    }

    fn put(&mut self, url_key: String, body: Vec<u8>, headers: String) {
        if body.len() > MAX_CACHEABLE_BODY_BYTES {
            return;
        }
        if let Some(pos) = self.entries.iter().position(|e| e.url_key == url_key) {
            let old = self.entries.remove(pos);
            self.total_bytes = self.total_bytes.saturating_sub(old.body.len());
        }
        while self.entries.len() >= MAX_CACHE_ENTRIES
            || self.total_bytes.saturating_add(body.len()) > MAX_CACHE_BYTES
        {
            if self.entries.is_empty() {
                break;
            }
            let old = self.entries.remove(0);
            self.total_bytes = self.total_bytes.saturating_sub(old.body.len());
        }
        self.total_bytes = self.total_bytes.saturating_add(body.len());
        self.entries.push(CacheEntry {
            url_key,
            body,
            headers,
            stored_secs: current_unix_secs(),
        });
    }
}

fn cache_get(url_key: &str) -> Option<(Vec<u8>, String)> {
    acquire(&CACHE_LOCK);
    let hit = unsafe {
        SUBRESOURCE_CACHE
            .as_ref()
            .and_then(|cache| cache.get(url_key))
    };
    release(&CACHE_LOCK);
    if hit.is_some() {
        return hit;
    }
    disk_cache_get(url_key).map(|(body, headers)| {
        cache_put_memory_only(String::from(url_key), body.clone(), headers.clone());
        (body, headers)
    })
}

fn cache_put(url_key: String, body: Vec<u8>, headers: String) {
    if body.is_empty() || !headers_cacheable(&headers) {
        return;
    }
    disk_cache_put(&url_key, &body, &headers);
    cache_put_memory_only(url_key, body, headers);
}

fn cache_put_memory_only(url_key: String, body: Vec<u8>, headers: String) {
    acquire(&CACHE_LOCK);
    unsafe {
        if let Some(cache) = SUBRESOURCE_CACHE.as_mut() {
            cache.put(url_key, body, headers);
        }
    }
    release(&CACHE_LOCK);
}

fn disk_cache_get(url_key: &str) -> Option<(Vec<u8>, String)> {
    let (body_path, headers_path) = disk_cache_paths(url_key);
    let headers = anyos_std::fs::read_to_string(&headers_path).ok()?;
    if !disk_cache_is_fresh(&body_path, &headers) {
        let _ = anyos_std::fs::unlink(&body_path);
        let _ = anyos_std::fs::unlink(&headers_path);
        return None;
    }
    let body = anyos_std::fs::read_to_vec(&body_path).ok()?;
    surf_net_log!("disk cache hit: {} ({} bytes)", url_key, body.len());
    Some((body, headers))
}

fn disk_cache_put(url_key: &str, body: &[u8], headers: &str) {
    if body.is_empty() || body.len() > MAX_DISK_CACHEABLE_BODY_BYTES || !headers_cacheable(headers)
    {
        return;
    }
    if !disk_cache_has_space_for(body.len()) {
        return;
    }

    let (body_path, headers_path) = disk_cache_paths(url_key);
    if !ensure_disk_cache_dir(url_key) {
        return;
    }
    if anyos_std::fs::write_bytes(&body_path, body).is_err() {
        return;
    }
    let _ = anyos_std::fs::write_bytes(&headers_path, headers.as_bytes());
}

fn ensure_disk_cache_dir(url_key: &str) -> bool {
    let root = disk_cache_root();
    if !ensure_dir_recursive(&root) {
        return false;
    }
    let mut dir = root;
    dir.push('/');
    append_hex64(&mut dir, hash_str64(url_key));
    ensure_dir_recursive(&dir)
}

fn disk_cache_paths(url_key: &str) -> (String, String) {
    let mut dir = disk_cache_root();
    dir.push('/');
    append_hex64(&mut dir, hash_str64(url_key));
    dir.push('/');
    let mut name = cache_file_name(url_key);
    if name.is_empty() {
        name.push_str("asset.bin");
    }
    if name.len() > 72 {
        name.truncate(72);
    }
    let mut body = dir;
    body.push_str(&name);
    let mut headers = body.clone();
    headers.push_str(".headers");
    (body, headers)
}

fn disk_cache_root() -> String {
    let mut home_buf = [0u8; 256];
    let home_len = anyos_std::env::get("HOME", &mut home_buf);
    let home = if home_len != u32::MAX && home_len > 0 {
        String::from(core::str::from_utf8(&home_buf[..home_len as usize]).unwrap_or("/Users/root"))
    } else {
        let uid = anyos_std::process::getuid();
        let mut name_buf = [0u8; 64];
        let name_len = anyos_std::process::getusername(uid, &mut name_buf);
        let username = if name_len != u32::MAX && name_len > 0 {
            core::str::from_utf8(&name_buf[..name_len as usize]).unwrap_or("root")
        } else {
            "root"
        };
        let mut fallback = String::from("/Users/");
        fallback.push_str(username);
        fallback
    };
    let mut root = String::from(home.trim_end_matches('/'));
    root.push_str("/.cache/surf/http");
    root
}

fn disk_cache_has_space_for(bytes: usize) -> bool {
    let mut candidate = disk_cache_root();
    let needed = (bytes as u64).saturating_add(DISK_CACHE_MIN_FREE_BYTES);
    loop {
        if let Some(stats) = anyos_std::fs::statfs(&candidate) {
            return stats.free_bytes >= needed;
        }
        if candidate == "/" {
            return true;
        }
        while candidate.len() > 1 && candidate.ends_with('/') {
            candidate.pop();
        }
        match candidate.rfind('/') {
            Some(0) | None => {
                candidate.truncate(1);
            }
            Some(idx) => candidate.truncate(idx),
        }
    }
}

fn ensure_dir_recursive(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let mut current = String::new();
    let mut saw_component = false;
    for part in path.split('/') {
        if part.is_empty() {
            if current.is_empty() {
                current.push('/');
            }
            continue;
        }
        if current.len() > 1 && !current.ends_with('/') {
            current.push('/');
        }
        current.push_str(part);
        saw_component = true;

        let mut stat = [0u32; 7];
        if anyos_std::fs::stat(&current, &mut stat) == 0 {
            if stat[0] != 1 {
                return false;
            }
            continue;
        }
        if anyos_std::fs::mkdir(&current) != 0 {
            let mut stat_after = [0u32; 7];
            if anyos_std::fs::stat(&current, &mut stat_after) != 0 || stat_after[0] != 1 {
                return false;
            }
        }
    }
    saw_component
}

fn cache_file_name(url_key: &str) -> String {
    let path_part = url_key
        .split('?')
        .next()
        .unwrap_or(url_key)
        .rsplit('/')
        .next()
        .unwrap_or("asset.bin");
    let mut out = String::new();
    for ch in path_part.chars() {
        let keep = ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_');
        out.push(if keep { ch } else { '_' });
    }
    if out.is_empty() || out == "." || out == ".." {
        String::from("asset.bin")
    } else {
        out
    }
}

fn disk_cache_is_fresh(body_path: &str, headers: &str) -> bool {
    let mut stat = [0u32; 7];
    if anyos_std::fs::stat(body_path, &mut stat) != 0 {
        return false;
    }
    cache_stored_response_is_fresh(stat[6], headers)
}

fn cache_stored_response_is_fresh(stored_secs: u32, headers: &str) -> bool {
    let max_age = match cache_max_age_secs(headers) {
        Some(age) => age,
        None if headers_cacheable(headers) => DISK_CACHE_DEFAULT_MAX_AGE_SECS,
        None => return false,
    };
    let now = current_unix_secs();
    now == 0 || stored_secs == 0 || now <= stored_secs || now.saturating_sub(stored_secs) <= max_age
}

fn headers_cacheable(headers: &str) -> bool {
    let cc = header_value(headers, "cache-control").unwrap_or_default();
    let lower = cc.to_ascii_lowercase();
    !lower.contains("no-store") && !lower.contains("no-cache") && !lower.contains("must-revalidate")
}

fn cache_max_age_secs(headers: &str) -> Option<u32> {
    let value = header_value(headers, "cache-control")?;
    for part in value.split(',') {
        let part = part.trim();
        let lower = part.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("max-age=") {
            if let Ok(v) = rest.parse::<u32>() {
                return Some(v);
            }
        }
    }
    None
}

fn header_value(headers: &str, name: &str) -> Option<String> {
    for line in headers.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case(name) {
                return Some(String::from(v.trim()));
            }
        }
    }
    None
}

fn current_unix_secs() -> u32 {
    let mut buf = [0u8; 8];
    if anyos_std::sys::time(&mut buf) == u32::MAX {
        return 0;
    }
    let year = u16::from_le_bytes([buf[0], buf[1]]) as i32;
    let month = buf[2] as i32;
    let day = buf[3] as i32;
    let hour = buf[4] as i32;
    let minute = buf[5] as i32;
    let second = buf[6] as i32;
    if year < 1970 || month < 1 || month > 12 || day < 1 || day > 31 {
        return 0;
    }
    let days = days_from_civil(year, month, day);
    let secs = days
        .saturating_mul(86_400)
        .saturating_add(hour.saturating_mul(3_600))
        .saturating_add(minute.saturating_mul(60))
        .saturating_add(second);
    if secs < 0 {
        0
    } else {
        secs as u32
    }
}

fn days_from_civil(year: i32, month: i32, day: i32) -> i32 {
    let y = year - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn hash_str64(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn append_hex64(out: &mut String, value: u64) {
    for shift in (0..64).step_by(4).rev() {
        let n = ((value >> shift) & 0xF) as u8;
        out.push(if n < 10 {
            (b'0' + n) as char
        } else {
            (b'a' + (n - 10)) as char
        });
    }
}

fn decode_image_in_worker(
    src: &str,
    body: &[u8],
    headers: &str,
    target_width: Option<u32>,
    target_height: Option<u32>,
) -> Option<DecodedRaster> {
    let is_svg = crate::resources::is_svg(src, headers);
    let decoded = if is_svg {
        crate::resources::decode_svg_to_image(body).map_err(|_| {
            surf_net_log!("worker svg decode failed: src={} bytes={}", src, body.len());
            libimage_client::ImageError::InvalidData
        })
    } else {
        crate::resources::decode_raster_to_image(body, target_width, target_height).map_err(|err| {
            surf_net_log!(
                "worker image decode failed: src={} bytes={} err={:?}",
                src,
                body.len(),
                err
            );
            err
        })
    };

    match decoded {
        Ok(image) => Some(DecodedRaster {
            pixels: image.pixels,
            width: image.width,
            height: image.height,
            format: image.format,
            suspicious_black_ppm: image.suspicious_black_ppm,
        }),
        Err(_) => None,
    }
}

// ═══════════════════════════════════════════════════════════
// Worker thread
// ═══════════════════════════════════════════════════════════

/// Entry point for the background network worker thread.
///
/// Exits after ~5 seconds of no work (1000 × 5ms sleep).  The next
/// `submit()` call will respawn the thread via `ensure_worker()`.
fn worker_entry_critical() {
    worker_entry(WorkerClass::Critical);
}

fn worker_entry_tls() {
    worker_entry(WorkerClass::Tls);
}

fn worker_entry_script() {
    worker_entry(WorkerClass::Script);
}

fn worker_entry_font() {
    worker_entry(WorkerClass::Font);
}

fn worker_entry_visible() {
    worker_entry(WorkerClass::Visible);
}

fn worker_entry_background() {
    worker_entry(WorkerClass::Background);
}

fn worker_entry(class: WorkerClass) {
    let mut pool = ConnPool::new();
    let mut idle_count: u32 = 0;

    loop {
        let req = dequeue_request(class);

        match req {
            Some(queued) => {
                idle_count = 0;
                REQUESTS_IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
                let host = request_host(&queued.req).to_string();
                let dequeued_ms = anyos_std::sys::uptime_ms();
                process_request(queued, dequeued_ms, &mut pool);
                release_host_slot(&host);
                REQUESTS_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
            }
            None => {
                idle_count += 1;
                if idle_count > 1000 {
                    // ~5 seconds idle — exit the thread.
                    // Must decrement the active count BEFORE exiting so
                    // ensure_worker() can respawn lanes on demand. Cannot
                    // `return` — the stack has no valid return address.
                    let remaining = worker_active_count(class)
                        .fetch_sub(1, Ordering::SeqCst)
                        .saturating_sub(1);
                    surf_net_log!(
                        "{} worker idle, exiting (remaining={})",
                        worker_label(class),
                        remaining
                    );
                    anyos_std::process::exit(0);
                }
                anyos_std::process::sleep(5);
            }
        }
    }
}

fn request_host(req: &FetchRequest) -> &str {
    match req {
        FetchRequest::Navigate { url, .. }
        | FetchRequest::NavigatePost { url, .. }
        | FetchRequest::Css { url, .. }
        | FetchRequest::Image { url, .. }
        | FetchRequest::Font { url, .. }
        | FetchRequest::Script { url, .. }
        | FetchRequest::ModuleScript { url, .. } => &url.host,
    }
}

fn try_acquire_host_slot(host: &str) -> bool {
    acquire(&HOST_LIMIT_LOCK);
    let ok = unsafe {
        let counts = HOST_ACTIVE_COUNTS.get_or_insert_with(Vec::new);
        if let Some(entry) = counts.iter_mut().find(|entry| entry.host == host) {
            if entry.count >= MAX_CONNECTIONS_PER_HOST {
                false
            } else {
                entry.count += 1;
                true
            }
        } else {
            counts.push(HostActiveCount {
                host: String::from(host),
                count: 1,
            });
            true
        }
    };
    release(&HOST_LIMIT_LOCK);
    ok
}

fn release_host_slot(host: &str) {
    acquire(&HOST_LIMIT_LOCK);
    unsafe {
        if let Some(counts) = HOST_ACTIVE_COUNTS.as_mut() {
            if let Some(pos) = counts.iter().position(|entry| entry.host == host) {
                if counts[pos].count <= 1 {
                    counts.remove(pos);
                } else {
                    counts[pos].count -= 1;
                }
            }
        }
    }
    release(&HOST_LIMIT_LOCK);
}

/// Dequeue the next request while respecting global per-host limits.
fn dequeue_request(class: WorkerClass) -> Option<QueuedFetchRequest> {
    let lock = request_lock(class);
    acquire(lock);
    let req = unsafe {
        if let Some(q) = request_queue_mut(class) {
            if q.is_empty() {
                None
            } else {
                let mut best: Option<(usize, u8)> = None;
                for (idx, item) in q.iter().enumerate() {
                    if !try_acquire_host_slot(request_host(&item.req)) {
                        continue;
                    }
                    let priority = request_priority(&item.req);
                    match best {
                        Some((_, best_priority)) if priority <= best_priority => {
                            release_host_slot(request_host(&item.req));
                        }
                        Some((best_idx, _)) => {
                            release_host_slot(request_host(&q[best_idx].req));
                            best = Some((idx, priority));
                        }
                        None => {
                            best = Some((idx, priority));
                        }
                    }
                }
                best.map(|(best_idx, _)| q.remove(best_idx))
            }
        } else {
            None
        }
    };
    release(lock);
    req
}

fn request_priority(req: &FetchRequest) -> u8 {
    match req {
        FetchRequest::Navigate { .. } | FetchRequest::NavigatePost { .. } => 5,
        FetchRequest::Css { .. } => 4,
        FetchRequest::Script { .. } | FetchRequest::ModuleScript { .. } => 3,
        FetchRequest::Font { .. } => 2,
        FetchRequest::Image { priority, .. } => match priority {
            ImagePriority::Viewport => 1,
            ImagePriority::Deferred => 0,
        },
    }
}

fn request_tab_index_mut(req: &mut FetchRequest) -> &mut usize {
    match req {
        FetchRequest::Navigate { tab_index, .. }
        | FetchRequest::NavigatePost { tab_index, .. }
        | FetchRequest::Css { tab_index, .. }
        | FetchRequest::Image { tab_index, .. }
        | FetchRequest::Font { tab_index, .. }
        | FetchRequest::Script { tab_index, .. }
        | FetchRequest::ModuleScript { tab_index, .. } => tab_index,
    }
}

/// Enqueue a result for the UI thread to pick up.
fn enqueue_result(mut result: FetchResult) {
    stamp_result_enqueued(&mut result);
    acquire(&RESULT_LOCK);
    unsafe {
        if let Some(mailboxes) = RESULT_MAILBOXES.as_mut() {
            let tab_index = result.tab_index();
            let Some(mailbox) = mailboxes.get_mut(tab_index) else {
                surf_net_log!("dropping result for out-of-range tab {}", tab_index);
                release(&RESULT_LOCK);
                return;
            };
            match &result {
                FetchResult::ScriptDone {
                    tab_index,
                    slot,
                    url,
                    body,
                    generation,
                    ..
                } => {
                    surf_net_log!(
                        "enqueue ScriptDone: tab={} slot={} url={}://{}{} bytes={} gen={} queue_before={}",
                        tab_index,
                        slot,
                        url.scheme,
                        url.host,
                        url.path,
                        body.len(),
                        generation,
                        mailbox.len()
                    );
                }
                _ => {}
            }
            mailbox.push(result);
        }
    }
    release(&RESULT_LOCK);
}

fn stamp_result_enqueued(result: &mut FetchResult) {
    let now = anyos_std::sys::uptime_ms();
    match result {
        FetchResult::NavDone { response, .. } => {
            response.timing.result_enqueued_ms = now;
        }
        FetchResult::CssDone {
            timing: Some(timing),
            ..
        }
        | FetchResult::ImageDone {
            timing: Some(timing),
            ..
        }
        | FetchResult::FontDone {
            timing: Some(timing),
            ..
        }
        | FetchResult::ScriptDone {
            timing: Some(timing),
            ..
        }
        | FetchResult::ModuleScriptDone {
            timing: Some(timing),
            ..
        } => {
            timing.result_enqueued_ms = now;
        }
        _ => {}
    }
}

/// Format a URL as a cache key string.
fn cache_key(url: &http::Url) -> String {
    let mut key = String::new();
    key.push_str(&url.scheme);
    key.push_str("://");
    key.push_str(&url.host);
    key.push(':');
    let port = url.port;
    if port >= 10000 {
        key.push((b'0' + (port / 10000 % 10) as u8) as char);
    }
    if port >= 1000 {
        key.push((b'0' + (port / 1000 % 10) as u8) as char);
    }
    if port >= 100 {
        key.push((b'0' + (port / 100 % 10) as u8) as char);
    }
    if port >= 10 {
        key.push((b'0' + (port / 10 % 10) as u8) as char);
    }
    key.push((b'0' + (port % 10) as u8) as char);
    key.push_str(&url.path);
    if url.host.eq_ignore_ascii_case("fonts.googleapis.com") {
        key.push_str("#ua=curl8");
    }
    key
}

fn stamp_worker_timing(
    timing: &mut http::RequestTiming,
    request_id: u32,
    submitted_ms: u32,
    dequeued_ms: u32,
) {
    timing.request_id = request_id;
    timing.submitted_ms = submitted_ms;
    timing.dequeued_ms = dequeued_ms;
    if timing.fetch_done_ms == 0 {
        timing.fetch_done_ms = anyos_std::sys::uptime_ms();
    }
}

fn instant_timing(request_id: u32, submitted_ms: u32, dequeued_ms: u32) -> http::RequestTiming {
    let now = anyos_std::sys::uptime_ms();
    http::RequestTiming {
        request_id,
        submitted_ms,
        dequeued_ms,
        start_ms: now,
        fetch_done_ms: now,
        ..http::RequestTiming::default()
    }
}

/// Process a single fetch request.
fn process_request(queued: QueuedFetchRequest, dequeued_ms: u32, pool: &mut ConnPool) {
    let current_gen = GENERATION.load(Ordering::Relaxed);
    let request_id = queued.request_id;
    let submitted_ms = queued.submitted_ms;
    let req = queued.req;

    match req {
        FetchRequest::Navigate {
            tab_index,
            url,
            mut cookies,
            generation,
        } => {
            surf_net_log!("navigate: {}://{}{}", url.scheme, url.host, url.path);

            match http::fetch(&url, &mut cookies, pool) {
                Ok(mut response) => {
                    stamp_worker_timing(
                        &mut response.timing,
                        request_id,
                        submitted_ms,
                        dequeued_ms,
                    );
                    enqueue_result(FetchResult::NavDone {
                        tab_index,
                        response,
                        url,
                        cookies,
                        generation,
                    });
                }
                Err(e) => {
                    enqueue_result(FetchResult::NavError {
                        tab_index,
                        error_msg: fetch_error_msg(e),
                        generation,
                    });
                }
            }
        }

        FetchRequest::NavigatePost {
            tab_index,
            url,
            body,
            mut cookies,
            generation,
        } => {
            surf_net_log!("navigate POST: {}://{}{}", url.scheme, url.host, url.path);

            match http::fetch_post(&url, &body, &mut cookies, pool) {
                Ok(mut response) => {
                    stamp_worker_timing(
                        &mut response.timing,
                        request_id,
                        submitted_ms,
                        dequeued_ms,
                    );
                    enqueue_result(FetchResult::NavDone {
                        tab_index,
                        response,
                        url,
                        cookies,
                        generation,
                    });
                }
                Err(e) => {
                    enqueue_result(FetchResult::NavError {
                        tab_index,
                        error_msg: fetch_error_msg(e),
                        generation,
                    });
                }
            }
        }

        FetchRequest::Css {
            tab_index,
            href,
            url,
            generation,
        } => {
            if generation != current_gen {
                return;
            }

            let key = cache_key(&url);

            // Check sub-resource cache first.
            if let Some((body_vec, headers_string)) = cache_get(&key) {
                surf_net_log!("CSS cache hit: {}", href);
                let css_text = crate::resources::decode_http_body(&body_vec, &headers_string);
                let css_text = crate::resources::resolve_css_resource_urls(&css_text, &url);
                let parsed = Some(DecodedCss {
                    sheet: libwebview::css::parse_stylesheet(&css_text),
                });
                enqueue_result(FetchResult::CssDone {
                    tab_index,
                    href,
                    url,
                    body: body_vec,
                    headers: headers_string,
                    parsed,
                    timing: Some(instant_timing(request_id, submitted_ms, dequeued_ms)),
                    from_cache: true,
                    generation,
                });
                return;
            }

            surf_net_log!("fetching CSS: {}", href);
            let mut css_cookies = CookieJar::new();
            match http::fetch(&url, &mut css_cookies, pool) {
                Ok(mut resp) if resp.status >= 200 && resp.status < 400 => {
                    stamp_worker_timing(&mut resp.timing, request_id, submitted_ms, dequeued_ms);
                    // Cache the response for future requests.
                    cache_put(key, resp.body.clone(), resp.headers.clone());
                    let css_text = crate::resources::decode_http_body(&resp.body, &resp.headers);
                    let css_text = crate::resources::resolve_css_resource_urls(&css_text, &url);
                    let parsed = Some(DecodedCss {
                        sheet: libwebview::css::parse_stylesheet(&css_text),
                    });
                    enqueue_result(FetchResult::CssDone {
                        tab_index,
                        href,
                        url,
                        body: resp.body,
                        headers: resp.headers,
                        parsed,
                        timing: Some(resp.timing),
                        from_cache: false,
                        generation,
                    });
                }
                Ok(mut resp) => {
                    stamp_worker_timing(&mut resp.timing, request_id, submitted_ms, dequeued_ms);
                    surf_net_log!("CSS fetch failed: {} ({})", href, key);
                    // Do not leave the page stuck waiting forever on a failed
                    // stylesheet; the UI thread still needs a completion signal
                    // so it can decrement `pending_stylesheet_count`.
                    enqueue_result(FetchResult::CssDone {
                        tab_index,
                        href,
                        url,
                        body: Vec::new(),
                        headers: resp.headers,
                        parsed: None,
                        timing: Some(resp.timing),
                        from_cache: false,
                        generation,
                    });
                }
                Err(_) => {
                    surf_net_log!("CSS fetch failed: {} ({})", href, key);
                    enqueue_result(FetchResult::CssDone {
                        tab_index,
                        href,
                        url,
                        body: Vec::new(),
                        headers: String::new(),
                        parsed: None,
                        timing: Some(instant_timing(request_id, submitted_ms, dequeued_ms)),
                        from_cache: false,
                        generation,
                    });
                }
            }
        }

        FetchRequest::Image {
            tab_index,
            src,
            url,
            target_width,
            target_height,
            priority,
            from_deferred,
            generation,
        } => {
            if generation != current_gen {
                return;
            }

            let key = cache_key(&url);

            // Check sub-resource cache first.
            if let Some((body_vec, headers_string)) = cache_get(&key) {
                surf_net_log!("image cache hit: {}", src);
                let encoded_len = body_vec.len();
                let decoded_raster = decode_image_in_worker(
                    &src,
                    &body_vec,
                    &headers_string,
                    target_width,
                    target_height,
                );
                enqueue_result(FetchResult::ImageDone {
                    tab_index,
                    src,
                    url,
                    encoded_len,
                    body: Vec::new(),
                    headers: headers_string,
                    decoded_raster,
                    priority,
                    from_deferred,
                    timing: Some(instant_timing(request_id, submitted_ms, dequeued_ms)),
                    from_cache: true,
                    generation,
                });
                return;
            }

            match http::fetch(&url, &mut CookieJar::new(), pool) {
                Ok(mut resp) if resp.status >= 200 && resp.status < 400 => {
                    stamp_worker_timing(&mut resp.timing, request_id, submitted_ms, dequeued_ms);
                    let is_svg = crate::resources::is_svg(&src, &resp.headers);
                    let encoded_len = resp.body.len();
                    if is_svg || encoded_len <= MAX_CACHEABLE_IMAGE_BODY_BYTES {
                        cache_put(key, resp.body.clone(), resp.headers.clone());
                    }
                    let decoded_raster = decode_image_in_worker(
                        &src,
                        &resp.body,
                        &resp.headers,
                        target_width,
                        target_height,
                    );
                    enqueue_result(FetchResult::ImageDone {
                        tab_index,
                        src,
                        url,
                        encoded_len,
                        body: Vec::new(),
                        headers: resp.headers,
                        decoded_raster,
                        priority,
                        from_deferred,
                        timing: Some(resp.timing),
                        from_cache: false,
                        generation,
                    });
                }
                Ok(mut resp) => {
                    stamp_worker_timing(&mut resp.timing, request_id, submitted_ms, dequeued_ms);
                    surf_net_log!(
                        "image fetch HTTP failure: status={} bytes={} src={}",
                        resp.status,
                        resp.body.len(),
                        src
                    );
                    enqueue_result(FetchResult::ImageDone {
                        tab_index,
                        src,
                        url,
                        body: Vec::new(),
                        encoded_len: resp.body.len(),
                        headers: resp.headers,
                        decoded_raster: None,
                        priority,
                        from_deferred,
                        timing: Some(resp.timing),
                        from_cache: false,
                        generation,
                    });
                }
                Err(e) => {
                    surf_net_log!("image fetch failed: {} ({})", src, fetch_error_msg(e));
                    enqueue_result(FetchResult::ImageDone {
                        tab_index,
                        src,
                        url,
                        body: Vec::new(),
                        encoded_len: 0,
                        headers: String::new(),
                        decoded_raster: None,
                        priority,
                        from_deferred,
                        timing: Some(instant_timing(request_id, submitted_ms, dequeued_ms)),
                        from_cache: false,
                        generation,
                    });
                }
            }
        }

        FetchRequest::Font {
            tab_index,
            family,
            weight,
            italic,
            url,
            display,
            generation,
        } => {
            if generation != current_gen {
                return;
            }

            let key = cache_key(&url);

            if let Some((body, _headers)) = cache_get(&key) {
                surf_net_log!("font cache hit: {}", family);
                enqueue_result(FetchResult::FontDone {
                    tab_index,
                    family,
                    weight,
                    italic,
                    url,
                    body,
                    display,
                    timing: Some(instant_timing(request_id, submitted_ms, dequeued_ms)),
                    from_cache: true,
                    generation,
                });
                return;
            }

            match http::fetch(&url, &mut CookieJar::new(), pool) {
                Ok(mut resp) if resp.status >= 200 && resp.status < 400 => {
                    stamp_worker_timing(&mut resp.timing, request_id, submitted_ms, dequeued_ms);
                    cache_put(key, resp.body.clone(), resp.headers.clone());
                    enqueue_result(FetchResult::FontDone {
                        tab_index,
                        family,
                        weight,
                        italic,
                        url,
                        body: resp.body,
                        display,
                        timing: Some(resp.timing),
                        from_cache: false,
                        generation,
                    });
                }
                Ok(mut resp) => {
                    stamp_worker_timing(&mut resp.timing, request_id, submitted_ms, dequeued_ms);
                    surf_net_log!(
                        "font fetch HTTP failure: status={} bytes={} family={}",
                        resp.status,
                        resp.body.len(),
                        family
                    );
                    enqueue_result(FetchResult::FontDone {
                        tab_index,
                        family,
                        weight,
                        italic,
                        url,
                        body: Vec::new(),
                        display,
                        timing: Some(resp.timing),
                        from_cache: false,
                        generation,
                    });
                }
                Err(e) => {
                    surf_net_log!("font fetch failed: {} ({})", family, fetch_error_msg(e));
                    enqueue_result(FetchResult::FontDone {
                        tab_index,
                        family,
                        weight,
                        italic,
                        url,
                        body: Vec::new(),
                        display,
                        timing: Some(instant_timing(request_id, submitted_ms, dequeued_ms)),
                        from_cache: false,
                        generation,
                    });
                }
            }
        }

        FetchRequest::Script {
            tab_index,
            slot,
            src,
            url,
            generation,
        } => {
            if generation != current_gen {
                return;
            }

            let key = cache_key(&url);

            if let Some((body, headers)) = cache_get(&key) {
                surf_net_log!("script cache hit: {}", src);
                enqueue_result(FetchResult::ScriptDone {
                    tab_index,
                    slot,
                    url,
                    body: body.to_vec(),
                    headers: String::from(headers),
                    timing: Some(instant_timing(request_id, submitted_ms, dequeued_ms)),
                    from_cache: true,
                    generation,
                });
                return;
            }

            surf_net_log!("fetching script: {}", src);
            match http::fetch(&url, &mut CookieJar::new(), pool) {
                Ok(mut resp) if resp.status >= 200 && resp.status < 400 => {
                    stamp_worker_timing(&mut resp.timing, request_id, submitted_ms, dequeued_ms);
                    surf_net_log!(
                        "script fetch OK: slot={} status={} bytes={} src={}",
                        slot,
                        resp.status,
                        resp.body.len(),
                        src
                    );
                    cache_put(key, resp.body.clone(), resp.headers.clone());
                    enqueue_result(FetchResult::ScriptDone {
                        tab_index,
                        slot,
                        url,
                        body: resp.body,
                        headers: resp.headers,
                        timing: Some(resp.timing),
                        from_cache: false,
                        generation,
                    });
                }
                Ok(mut resp) => {
                    stamp_worker_timing(&mut resp.timing, request_id, submitted_ms, dequeued_ms);
                    surf_net_log!(
                        "script fetch HTTP failure: slot={} status={} bytes={} src={}",
                        slot,
                        resp.status,
                        resp.body.len(),
                        src
                    );
                    enqueue_result(FetchResult::ScriptDone {
                        tab_index,
                        slot,
                        url,
                        body: Vec::new(),
                        headers: resp.headers,
                        timing: Some(resp.timing),
                        from_cache: false,
                        generation,
                    });
                }
                _ => {
                    surf_net_log!("script fetch failed: {} ({})", src, key);
                    // Send an empty body so the slot is filled and JS execution
                    // is not blocked forever waiting for this slot.
                    enqueue_result(FetchResult::ScriptDone {
                        tab_index,
                        slot,
                        url,
                        body: Vec::new(),
                        headers: String::new(),
                        timing: Some(instant_timing(request_id, submitted_ms, dequeued_ms)),
                        from_cache: false,
                        generation,
                    });
                }
            }
        }

        FetchRequest::ModuleScript {
            tab_index,
            specifier,
            url,
            generation,
        } => {
            if generation != current_gen {
                return;
            }

            let key = cache_key(&url);

            if let Some((body, headers)) = cache_get(&key) {
                surf_net_log!("module script cache hit: {}", specifier);
                enqueue_result(FetchResult::ModuleScriptDone {
                    tab_index,
                    specifier,
                    url,
                    body: body.to_vec(),
                    headers: String::from(headers),
                    timing: Some(instant_timing(request_id, submitted_ms, dequeued_ms)),
                    from_cache: true,
                    generation,
                });
                return;
            }

            surf_net_log!("fetching module script: {}", specifier);
            match http::fetch(&url, &mut CookieJar::new(), pool) {
                Ok(mut resp) if resp.status >= 200 && resp.status < 400 => {
                    stamp_worker_timing(&mut resp.timing, request_id, submitted_ms, dequeued_ms);
                    cache_put(key, resp.body.clone(), resp.headers.clone());
                    enqueue_result(FetchResult::ModuleScriptDone {
                        tab_index,
                        specifier,
                        url,
                        body: resp.body,
                        headers: resp.headers,
                        timing: Some(resp.timing),
                        from_cache: false,
                        generation,
                    });
                }
                Ok(mut resp) => {
                    stamp_worker_timing(&mut resp.timing, request_id, submitted_ms, dequeued_ms);
                    surf_net_log!(
                        "module script HTTP failure: status={} bytes={} specifier={}",
                        resp.status,
                        resp.body.len(),
                        specifier
                    );
                    enqueue_result(FetchResult::ModuleScriptDone {
                        tab_index,
                        specifier,
                        url,
                        body: Vec::new(),
                        headers: resp.headers,
                        timing: Some(resp.timing),
                        from_cache: false,
                        generation,
                    });
                }
                Err(e) => {
                    surf_net_log!(
                        "module script fetch failed: {} ({})",
                        specifier,
                        fetch_error_msg(e)
                    );
                    enqueue_result(FetchResult::ModuleScriptDone {
                        tab_index,
                        specifier,
                        url,
                        body: Vec::new(),
                        headers: String::new(),
                        timing: Some(instant_timing(request_id, submitted_ms, dequeued_ms)),
                        from_cache: false,
                        generation,
                    });
                }
            }
        }
    }
}

/// Map a `FetchError` to a static error message string.
fn fetch_error_msg(e: FetchError) -> &'static str {
    match e {
        FetchError::InvalidUrl => "Invalid URL",
        FetchError::DnsFailure => "DNS lookup failed",
        FetchError::ConnectFailure => "Connection failed",
        FetchError::SendFailure => "Send failed",
        FetchError::NoResponse => "No response",
        FetchError::TooManyRedirects => "Too many redirects",
        FetchError::TlsHandshakeFailed => "TLS handshake failed",
    }
}
