// Copyright (c) 2024-2026 Christian Moeller
// SPDX-License-Identifier: MIT

//! Background network worker thread for the Surf browser.
//!
//! Moves all blocking HTTP fetches (navigation, CSS, images) off the
//! UI thread onto a dedicated worker.  Communication uses static shared
//! state guarded by `AtomicBool` spinlocks since `Thread::spawn` only
//! accepts `fn()` (not closures).

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::http::{self, ConnPool, CookieJar, FetchError, Url};

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
        priority: ImagePriority,
        generation: u32,
    },
    /// Web font fetch (@font-face src).
    Font {
        tab_index: usize,
        family: String,
        url: Url,
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
        body: Vec<u8>,
        headers: String,
        generation: u32,
    },
    /// Image fetch completed successfully.
    ImageDone {
        tab_index: usize,
        src: String,
        body: Vec<u8>,
        headers: String,
        priority: ImagePriority,
        generation: u32,
    },
    /// Web font fetch completed successfully.
    FontDone {
        tab_index: usize,
        family: String,
        body: Vec<u8>,
        generation: u32,
    },
    /// External script fetch completed successfully.
    ScriptDone {
        tab_index: usize,
        /// Slot index — matches the request's `slot` so the UI thread can
        /// place the fetched text at the correct position in document order.
        slot: usize,
        body: Vec<u8>,
        headers: String,
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
            | FetchResult::ScriptDone { tab_index, .. } => *tab_index,
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
const RESULT_MAILBOX_COUNT: usize = 16;

static mut REQUEST_QUEUE_CRITICAL: Option<Vec<FetchRequest>> = None;
static mut REQUEST_QUEUE_TLS: Option<Vec<FetchRequest>> = None;
static mut REQUEST_QUEUE_SCRIPT: Option<Vec<FetchRequest>> = None;
static mut REQUEST_QUEUE_FONT: Option<Vec<FetchRequest>> = None;
static mut REQUEST_QUEUE_VISIBLE: Option<Vec<FetchRequest>> = None;
static mut REQUEST_QUEUE_BACKGROUND: Option<Vec<FetchRequest>> = None;
static mut RESULT_MAILBOXES: Option<Vec<Vec<FetchResult>>> = None;

/// Generation counter — incremented on each Navigate request.
/// Worker skips CSS/Image requests with a stale generation.
static GENERATION: AtomicU32 = AtomicU32::new(0);

/// Whether the worker thread has been started.
static WORKER_STARTED_CRITICAL: AtomicBool = AtomicBool::new(false);
static WORKER_STARTED_TLS: AtomicBool = AtomicBool::new(false);
static WORKER_STARTED_SCRIPT: AtomicBool = AtomicBool::new(false);
static WORKER_STARTED_FONT: AtomicBool = AtomicBool::new(false);
static WORKER_STARTED_VISIBLE: AtomicBool = AtomicBool::new(false);
static WORKER_STARTED_BACKGROUND: AtomicBool = AtomicBool::new(false);

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

fn worker_started_flag(class: WorkerClass) -> &'static AtomicBool {
    match class {
        WorkerClass::Critical => &WORKER_STARTED_CRITICAL,
        WorkerClass::Tls => &WORKER_STARTED_TLS,
        WorkerClass::Script => &WORKER_STARTED_SCRIPT,
        WorkerClass::Font => &WORKER_STARTED_FONT,
        WorkerClass::Visible => &WORKER_STARTED_VISIBLE,
        WorkerClass::Background => &WORKER_STARTED_BACKGROUND,
    }
}

unsafe fn request_queue_mut(class: WorkerClass) -> Option<&'static mut Vec<FetchRequest>> {
    match class {
        WorkerClass::Critical => REQUEST_QUEUE_CRITICAL.as_mut(),
        WorkerClass::Tls => REQUEST_QUEUE_TLS.as_mut(),
        WorkerClass::Script => REQUEST_QUEUE_SCRIPT.as_mut(),
        WorkerClass::Font => REQUEST_QUEUE_FONT.as_mut(),
        WorkerClass::Visible => REQUEST_QUEUE_VISIBLE.as_mut(),
        WorkerClass::Background => REQUEST_QUEUE_BACKGROUND.as_mut(),
    }
}

unsafe fn request_queue_ref(class: WorkerClass) -> Option<&'static Vec<FetchRequest>> {
    match class {
        WorkerClass::Critical => REQUEST_QUEUE_CRITICAL.as_ref(),
        WorkerClass::Tls => REQUEST_QUEUE_TLS.as_ref(),
        WorkerClass::Script => REQUEST_QUEUE_SCRIPT.as_ref(),
        WorkerClass::Font => REQUEST_QUEUE_FONT.as_ref(),
        WorkerClass::Visible => REQUEST_QUEUE_VISIBLE.as_ref(),
        WorkerClass::Background => REQUEST_QUEUE_BACKGROUND.as_ref(),
    }
}

fn request_worker_class(req: &FetchRequest) -> WorkerClass {
    if request_is_https(req) {
        return WorkerClass::Tls;
    }
    match req {
        FetchRequest::Navigate { .. }
        | FetchRequest::NavigatePost { .. }
        | FetchRequest::Css { .. } => WorkerClass::Critical,
        FetchRequest::Script { .. } => WorkerClass::Script,
        FetchRequest::Font { .. } => WorkerClass::Font,
        FetchRequest::Image { priority, .. } => match priority {
            ImagePriority::Viewport => WorkerClass::Visible,
            ImagePriority::Deferred => WorkerClass::Background,
        },
    }
}

fn request_is_https(req: &FetchRequest) -> bool {
    match req {
        FetchRequest::Navigate { url, .. }
        | FetchRequest::NavigatePost { url, .. }
        | FetchRequest::Css { url, .. }
        | FetchRequest::Image { url, .. }
        | FetchRequest::Font { url, .. }
        | FetchRequest::Script { url, .. } => url.scheme == "https",
    }
}

fn ensure_worker(class: WorkerClass) {
    let started = worker_started_flag(class);
    if started.load(Ordering::Relaxed) {
        return;
    }
    if started
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
        .is_ok()
    {
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
                surf_net_log!("{} worker thread started", worker_label(class));
            }
            Err(_) => {
                surf_net_log!(
                    "ERROR: failed to spawn {} worker thread",
                    worker_label(class)
                );
                started.store(false, Ordering::SeqCst);
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
    }
}

/// Submit a request to the worker queue.
pub(crate) fn submit(req: FetchRequest) {
    let class = request_worker_class(&req);
    ensure_worker(class);
    let lock = request_lock(class);
    acquire(lock);
    unsafe {
        if let Some(q) = request_queue_mut(class) {
            q.push(req);
        }
    }
    release(lock);
}

/// Drain completed results for one tab from its mailbox.
pub(crate) fn drain_results_for_tab(tab_index: usize) -> Vec<FetchResult> {
    acquire(&RESULT_LOCK);
    let results = unsafe {
        if let Some(mailboxes) = RESULT_MAILBOXES.as_mut() {
            if let Some(mailbox) = mailboxes.get_mut(tab_index) {
                surf_net_log!("mailbox poll: tab={} pending={}", tab_index, mailbox.len());
                if mailbox.is_empty() {
                    Vec::new()
                } else {
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
                q.retain(|r| match r {
                    FetchRequest::Navigate { .. } | FetchRequest::NavigatePost { .. } => true,
                    FetchRequest::Css { generation, .. }
                    | FetchRequest::Image { generation, .. }
                    | FetchRequest::Font { generation, .. }
                    | FetchRequest::Script { generation, .. } => *generation == gen,
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
                    | FetchResult::ScriptDone { generation, .. } => *generation == gen,
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
                for mut req in old.drain(..) {
                    let tab_index = request_tab_index_mut(&mut req);
                    if *tab_index == closed_idx {
                        continue;
                    }
                    if *tab_index > closed_idx {
                        *tab_index -= 1;
                    }
                    q.push(req);
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
}

/// Best-effort signal for the UI thread that the network worker may still
/// produce results soon.
pub(crate) fn has_pending_activity() -> bool {
    if WORKER_STARTED_CRITICAL.load(Ordering::Relaxed)
        || WORKER_STARTED_TLS.load(Ordering::Relaxed)
        || WORKER_STARTED_SCRIPT.load(Ordering::Relaxed)
        || WORKER_STARTED_FONT.load(Ordering::Relaxed)
        || WORKER_STARTED_VISIBLE.load(Ordering::Relaxed)
        || WORKER_STARTED_BACKGROUND.load(Ordering::Relaxed)
    {
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

/// Maximum number of cached CSS/image responses.
const MAX_CACHE_ENTRIES: usize = 128;

/// A cached HTTP response for a sub-resource (CSS or image).
struct CacheEntry {
    /// Cache key: fully-qualified URL string.
    url_key: String,
    /// Raw response body.
    body: Vec<u8>,
    /// Response headers.
    headers: String,
}

/// Per-worker sub-resource cache for CSS and images.
///
/// Avoids re-fetching the same stylesheet or image across page loads.
/// Uses a simple FIFO eviction when the cache is full.
struct SubResourceCache {
    entries: Vec<CacheEntry>,
}

impl SubResourceCache {
    fn new() -> Self {
        SubResourceCache {
            entries: Vec::new(),
        }
    }

    /// Look up a cached response by URL.
    fn get(&self, url_key: &str) -> Option<(&[u8], &str)> {
        self.entries
            .iter()
            .find(|e| e.url_key == url_key)
            .map(|e| (e.body.as_slice(), e.headers.as_str()))
    }

    /// Store a response in the cache, evicting the oldest entry if full.
    fn put(&mut self, url_key: String, body: Vec<u8>, headers: String) {
        if self.entries.len() >= MAX_CACHE_ENTRIES {
            self.entries.remove(0);
        }
        self.entries.push(CacheEntry {
            url_key,
            body,
            headers,
        });
    }

    /// Clear all entries (called on navigation to a new page).
    fn _clear(&mut self) {
        self.entries.clear();
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
    let mut cache = SubResourceCache::new();
    let mut idle_count: u32 = 0;

    loop {
        let req = dequeue_request(class);

        match req {
            Some(request) => {
                idle_count = 0;
                process_request(request, &mut pool, &mut cache);
            }
            None => {
                idle_count += 1;
                if idle_count > 1000 {
                    // ~5 seconds idle — exit the thread.
                    // Must store false BEFORE exiting so ensure_worker() can
                    // respawn.  Cannot `return` — the stack has no valid
                    // return address (mmap zeroes it), so RIP would become 0.
                    worker_started_flag(class).store(false, Ordering::SeqCst);
                    surf_net_log!("{} worker idle, exiting", worker_label(class));
                    anyos_std::process::exit(0);
                }
                anyos_std::process::sleep(5);
            }
        }
    }
}

/// Dequeue the next request from the queue (FIFO).
fn dequeue_request(class: WorkerClass) -> Option<FetchRequest> {
    let lock = request_lock(class);
    acquire(lock);
    let req = unsafe {
        if let Some(q) = request_queue_mut(class) {
            if q.is_empty() {
                None
            } else {
                let mut best_idx = 0usize;
                let mut best_priority = request_priority(&q[0]);
                for (idx, item) in q.iter().enumerate().skip(1) {
                    let priority = request_priority(item);
                    if priority > best_priority {
                        best_priority = priority;
                        best_idx = idx;
                    }
                }
                Some(q.remove(best_idx))
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
        FetchRequest::Script { .. } => 3,
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
        | FetchRequest::Script { tab_index, .. } => tab_index,
    }
}

/// Enqueue a result for the UI thread to pick up.
fn enqueue_result(result: FetchResult) {
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
                    body,
                    generation,
                    ..
                } => {
                    surf_net_log!(
                        "enqueue ScriptDone: tab={} slot={} bytes={} gen={} queue_before={}",
                        tab_index,
                        slot,
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
    key
}

/// Process a single fetch request.
fn process_request(req: FetchRequest, pool: &mut ConnPool, cache: &mut SubResourceCache) {
    let current_gen = GENERATION.load(Ordering::Relaxed);

    match req {
        FetchRequest::Navigate {
            tab_index,
            url,
            mut cookies,
            generation,
        } => {
            surf_net_log!("navigate: {}://{}{}", url.scheme, url.host, url.path);

            match http::fetch(&url, &mut cookies, pool) {
                Ok(response) => {
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
                Ok(response) => {
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
            if let Some((body, headers)) = cache.get(&key) {
                surf_net_log!("CSS cache hit: {}", href);
                enqueue_result(FetchResult::CssDone {
                    tab_index,
                    href,
                    body: body.to_vec(),
                    headers: String::from(headers),
                    generation,
                });
                return;
            }

            surf_net_log!("fetching CSS: {}", href);
            let mut css_cookies = CookieJar::new();
            match http::fetch(&url, &mut css_cookies, pool) {
                Ok(resp) if resp.status >= 200 && resp.status < 400 => {
                    // Cache the response for future requests.
                    cache.put(key, resp.body.clone(), resp.headers.clone());
                    enqueue_result(FetchResult::CssDone {
                        tab_index,
                        href,
                        body: resp.body,
                        headers: resp.headers,
                        generation,
                    });
                }
                _ => {
                    surf_net_log!("CSS fetch failed: {} ({})", href, key);
                    // Do not leave the page stuck waiting forever on a failed
                    // stylesheet; the UI thread still needs a completion signal
                    // so it can decrement `pending_stylesheet_count`.
                    enqueue_result(FetchResult::CssDone {
                        tab_index,
                        href,
                        body: Vec::new(),
                        headers: String::new(),
                        generation,
                    });
                }
            }
        }

        FetchRequest::Image {
            tab_index,
            src,
            url,
            priority,
            generation,
        } => {
            if generation != current_gen {
                return;
            }

            let key = cache_key(&url);

            // Check sub-resource cache first.
            if let Some((body, headers)) = cache.get(&key) {
                surf_net_log!("image cache hit: {}", src);
                enqueue_result(FetchResult::ImageDone {
                    tab_index,
                    src,
                    body: body.to_vec(),
                    headers: String::from(headers),
                    priority,
                    generation,
                });
                return;
            }

            match http::fetch(&url, &mut CookieJar::new(), pool) {
                Ok(resp) => {
                    cache.put(key, resp.body.clone(), resp.headers.clone());
                    enqueue_result(FetchResult::ImageDone {
                        tab_index,
                        src,
                        body: resp.body,
                        headers: resp.headers,
                        priority,
                        generation,
                    });
                }
                _ => {}
            }
        }

        FetchRequest::Font {
            tab_index,
            family,
            url,
            generation,
        } => {
            if generation != current_gen {
                return;
            }

            match http::fetch(&url, &mut CookieJar::new(), pool) {
                Ok(resp) => {
                    enqueue_result(FetchResult::FontDone {
                        tab_index,
                        family,
                        body: resp.body,
                        generation,
                    });
                }
                _ => {}
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

            if let Some((body, headers)) = cache.get(&key) {
                surf_net_log!("script cache hit: {}", src);
                enqueue_result(FetchResult::ScriptDone {
                    tab_index,
                    slot,
                    body: body.to_vec(),
                    headers: String::from(headers),
                    generation,
                });
                return;
            }

            surf_net_log!("fetching script: {}", src);
            match http::fetch(&url, &mut CookieJar::new(), pool) {
                Ok(resp) if resp.status >= 200 && resp.status < 400 => {
                    surf_net_log!(
                        "script fetch OK: slot={} status={} bytes={} src={}",
                        slot,
                        resp.status,
                        resp.body.len(),
                        src
                    );
                    cache.put(key, resp.body.clone(), resp.headers.clone());
                    enqueue_result(FetchResult::ScriptDone {
                        tab_index,
                        slot,
                        body: resp.body,
                        headers: resp.headers,
                        generation,
                    });
                }
                Ok(resp) => {
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
                        body: Vec::new(),
                        headers: resp.headers,
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
                        body: Vec::new(),
                        headers: String::new(),
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
