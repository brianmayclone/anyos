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

// ═══════════════════════════════════════════════════════════
// Request / result types
// ═══════════════════════════════════════════════════════════

/// A fetch request submitted by the UI thread.
pub(crate) enum FetchRequest {
    /// Full page navigation (GET): fetch HTML, return body + headers + cookies.
    Navigate {
        url: Url,
        cookies: CookieJar,
        generation: u32,
    },
    /// Full page navigation (POST): fetch HTML with form body.
    NavigatePost {
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

/// A completed fetch result returned by the worker thread.
pub(crate) enum FetchResult {
    /// Navigation completed successfully.
    NavDone {
        response: http::Response,
        url: Url,
        cookies: CookieJar,
        generation: u32,
    },
    /// Navigation failed.
    NavError {
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

// ═══════════════════════════════════════════════════════════
// Shared state + spinlock
// ═══════════════════════════════════════════════════════════

static REQUEST_LOCK: AtomicBool = AtomicBool::new(false);
static RESULT_LOCK: AtomicBool = AtomicBool::new(false);

static mut REQUEST_QUEUE: Option<Vec<FetchRequest>> = None;
static mut RESULT_QUEUE: Option<Vec<FetchResult>> = None;

/// Generation counter — incremented on each Navigate request.
/// Worker skips CSS/Image requests with a stale generation.
static GENERATION: AtomicU32 = AtomicU32::new(0);

/// Whether the worker thread has been started.
static WORKER_STARTED: AtomicBool = AtomicBool::new(false);

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

// ═══════════════════════════════════════════════════════════
// Public API (called from UI thread)
// ═══════════════════════════════════════════════════════════

/// Initialize the shared queues. Must be called once from the UI thread
/// before any requests are submitted.
pub(crate) fn init() {
    unsafe {
        REQUEST_QUEUE = Some(Vec::new());
        RESULT_QUEUE = Some(Vec::new());
    }
}

/// Ensure the worker thread is running. Spawns it on first call.
pub(crate) fn ensure_worker() {
    if WORKER_STARTED.load(Ordering::Relaxed) {
        return;
    }
    if WORKER_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
        .is_ok()
    {
        // 256 KiB stack — HTTP recv uses 32 KiB buffers on stack, plus
        // BearSSL TLS state; the default 64 KiB overflows.
        match anyos_std::process::Thread::spawn_with_stack(worker_entry, 256 * 1024, "surf-net") {
            Ok(handle) => {
                // Detach: the worker runs for the lifetime of the process.
                // We intentionally leak the handle to avoid joining on drop.
                core::mem::forget(handle);
                anyos_std::println!("[surf-net] worker thread started");
            }
            Err(_) => {
                anyos_std::println!("[surf-net] ERROR: failed to spawn worker thread");
                WORKER_STARTED.store(false, Ordering::SeqCst);
            }
        }
    }
}

/// Submit a request to the worker queue.
pub(crate) fn submit(req: FetchRequest) {
    ensure_worker();
    acquire(&REQUEST_LOCK);
    unsafe {
        if let Some(q) = REQUEST_QUEUE.as_mut() {
            q.push(req);
        }
    }
    release(&REQUEST_LOCK);
}

/// Drain all completed results from the result queue.
/// Returns an empty Vec if nothing is ready yet.
pub(crate) fn drain_results() -> Vec<FetchResult> {
    acquire(&RESULT_LOCK);
    let results = unsafe {
        if let Some(q) = RESULT_QUEUE.as_mut() {
            if q.is_empty() {
                Vec::new()
            } else {
                core::mem::replace(q, Vec::new())
            }
        } else {
            Vec::new()
        }
    };
    release(&RESULT_LOCK);
    results
}

/// Bump the generation counter and clear any pending CSS/Image requests
/// from previous pages. Called when a new Navigate begins.
pub(crate) fn new_generation() -> u32 {
    let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    // Clear pending resource requests for old generations.
    acquire(&REQUEST_LOCK);
    unsafe {
        if let Some(q) = REQUEST_QUEUE.as_mut() {
            q.retain(|r| match r {
                FetchRequest::Navigate { .. } | FetchRequest::NavigatePost { .. } => true,
                FetchRequest::Css { generation, .. }
                | FetchRequest::Image { generation, .. }
                | FetchRequest::Font { generation, .. }
                | FetchRequest::Script { generation, .. } => *generation == gen,
            });
        }
    }
    release(&REQUEST_LOCK);
    // Also clear stale results.
    acquire(&RESULT_LOCK);
    unsafe {
        if let Some(q) = RESULT_QUEUE.as_mut() {
            q.retain(|r| match r {
                FetchResult::NavDone { .. } | FetchResult::NavError { .. } => true,
                FetchResult::CssDone { generation, .. }
                | FetchResult::ImageDone { generation, .. }
                | FetchResult::FontDone { generation, .. }
                | FetchResult::ScriptDone { generation, .. } => *generation == gen,
            });
        }
    }
    release(&RESULT_LOCK);
    gen
}

/// Get the current generation counter.
pub(crate) fn current_generation() -> u32 {
    GENERATION.load(Ordering::Relaxed)
}

/// Best-effort signal for the UI thread that the network worker may still
/// produce results soon.
pub(crate) fn has_pending_activity() -> bool {
    if WORKER_STARTED.load(Ordering::Relaxed) {
        return true;
    }

    acquire(&REQUEST_LOCK);
    let request_non_empty = unsafe { REQUEST_QUEUE.as_ref().is_some_and(|q| !q.is_empty()) };
    release(&REQUEST_LOCK);
    if request_non_empty {
        return true;
    }

    acquire(&RESULT_LOCK);
    let result_non_empty = unsafe { RESULT_QUEUE.as_ref().is_some_and(|q| !q.is_empty()) };
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
fn worker_entry() {
    let mut pool = ConnPool::new();
    let mut cache = SubResourceCache::new();
    let mut idle_count: u32 = 0;

    loop {
        let req = dequeue_request();

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
                    WORKER_STARTED.store(false, Ordering::SeqCst);
                    anyos_std::println!("[surf-net] worker idle, exiting");
                    anyos_std::process::exit(0);
                }
                anyos_std::process::sleep(5);
            }
        }
    }
}

/// Dequeue the next request from the queue (FIFO).
fn dequeue_request() -> Option<FetchRequest> {
    acquire(&REQUEST_LOCK);
    let req = unsafe {
        if let Some(q) = REQUEST_QUEUE.as_mut() {
            if q.is_empty() {
                None
            } else {
                Some(q.remove(0))
            }
        } else {
            None
        }
    };
    release(&REQUEST_LOCK);
    req
}

/// Enqueue a result for the UI thread to pick up.
fn enqueue_result(result: FetchResult) {
    acquire(&RESULT_LOCK);
    unsafe {
        if let Some(q) = RESULT_QUEUE.as_mut() {
            match &result {
                FetchResult::ScriptDone {
                    tab_index,
                    slot,
                    body,
                    generation,
                    ..
                } => {
                    anyos_std::println!(
                        "[surf-net] enqueue ScriptDone: tab={} slot={} bytes={} gen={} queue_before={}",
                        tab_index,
                        slot,
                        body.len(),
                        generation,
                        q.len()
                    );
                }
                _ => {}
            }
            q.push(result);
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
            url,
            mut cookies,
            generation,
        } => {
            anyos_std::println!(
                "[surf-net] navigate: {}://{}{}",
                url.scheme,
                url.host,
                url.path
            );

            match http::fetch(&url, &mut cookies, pool) {
                Ok(response) => {
                    enqueue_result(FetchResult::NavDone {
                        response,
                        url,
                        cookies,
                        generation,
                    });
                }
                Err(e) => {
                    enqueue_result(FetchResult::NavError {
                        error_msg: fetch_error_msg(e),
                        generation,
                    });
                }
            }
        }

        FetchRequest::NavigatePost {
            url,
            body,
            mut cookies,
            generation,
        } => {
            anyos_std::println!(
                "[surf-net] navigate POST: {}://{}{}",
                url.scheme,
                url.host,
                url.path
            );

            match http::fetch_post(&url, &body, &mut cookies, pool) {
                Ok(response) => {
                    enqueue_result(FetchResult::NavDone {
                        response,
                        url,
                        cookies,
                        generation,
                    });
                }
                Err(e) => {
                    enqueue_result(FetchResult::NavError {
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
                anyos_std::println!("[surf-net] CSS cache hit: {}", href);
                enqueue_result(FetchResult::CssDone {
                    tab_index,
                    href,
                    body: body.to_vec(),
                    headers: String::from(headers),
                    generation,
                });
                return;
            }

            anyos_std::println!("[surf-net] fetching CSS: {}", href);
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
                    anyos_std::println!("[surf-net] CSS fetch failed: {} ({})", href, key);
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
            generation,
        } => {
            if generation != current_gen {
                return;
            }

            let key = cache_key(&url);

            // Check sub-resource cache first.
            if let Some((body, headers)) = cache.get(&key) {
                anyos_std::println!("[surf-net] image cache hit: {}", src);
                enqueue_result(FetchResult::ImageDone {
                    tab_index,
                    src,
                    body: body.to_vec(),
                    headers: String::from(headers),
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
                anyos_std::println!("[surf-net] script cache hit: {}", src);
                enqueue_result(FetchResult::ScriptDone {
                    tab_index,
                    slot,
                    body: body.to_vec(),
                    headers: String::from(headers),
                    generation,
                });
                return;
            }

            anyos_std::println!("[surf-net] fetching script: {}", src);
            match http::fetch(&url, &mut CookieJar::new(), pool) {
                Ok(resp) if resp.status >= 200 && resp.status < 400 => {
                    anyos_std::println!(
                        "[surf-net] script fetch OK: slot={} status={} bytes={} src={}",
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
                    anyos_std::println!(
                        "[surf-net] script fetch HTTP failure: slot={} status={} bytes={} src={}",
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
                    anyos_std::println!("[surf-net] script fetch failed: {} ({})", src, key);
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
