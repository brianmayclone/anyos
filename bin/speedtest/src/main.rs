#![no_std]
#![no_main]

use anyos_std::{print, println, Vec};
use anyos_std::{args, net, process, sys};

anyos_std::entry!(main);

const URL_10MB: &str = "http://speedtest.tele2.net/10MB.zip";
const URL_100MB: &str = "http://speedtest.tele2.net/100MB.zip";
const VERSION: &str = "1.0";

/// ~512 KB per sample chunk
const CHUNK_SIZE: u32 = 512 * 1024;
/// Max samples we track
const MAX_SAMPLES: usize = 512;

// ── Global state for progress callback ─────────────────────────────────────

struct ChunkSample {
    bytes: u32,
    elapsed_ms: u32,
}

struct SpeedState {
    /// Timestamp when download started
    start_ms: u32,
    /// Bytes received at last chunk boundary
    chunk_start_bytes: u32,
    /// Timestamp at last chunk boundary
    chunk_start_ms: u32,
    /// Collected chunk samples
    samples: Vec<ChunkSample>,
    /// Last progress display threshold (MB)
    last_progress_mb: u32,
    /// Total bytes reported by server (Content-Length)
    total_size: u32,
    /// Actual bytes received (last received value from callback)
    received: u32,
}

static mut STATE: Option<SpeedState> = None;
fn state() -> &'static mut SpeedState {
    unsafe { STATE.as_mut().unwrap() }
}

extern "C" fn progress_callback(received: u32, total: u32, _userdata: u64) {
    let s = state();

    // Track actual received bytes and Content-Length
    s.received = received;
    if s.total_size == 0 && total > 0 {
        s.total_size = total;
    }

    // How many bytes since last chunk boundary
    let chunk_bytes = received - s.chunk_start_bytes;

    // Emit sample when chunk is full
    if chunk_bytes >= CHUNK_SIZE && s.samples.len() < MAX_SAMPLES {
        let now = sys::uptime_ms();
        let elapsed = now.wrapping_sub(s.chunk_start_ms);
        if elapsed > 0 {
            s.samples.push(ChunkSample {
                bytes: chunk_bytes,
                elapsed_ms: elapsed,
            });
        }
        s.chunk_start_bytes = received;
        s.chunk_start_ms = now;
    }

    // Progress display every 1 MB
    let mb = received / (1024 * 1024);
    if mb > s.last_progress_mb {
        s.last_progress_mb = mb;
        let now = sys::uptime_ms();
        let total_elapsed = now.wrapping_sub(s.start_ms);
        let speed_kbps = if total_elapsed > 0 {
            (received as u64 * 8 * 1000) / (total_elapsed as u64 * 1024)
        } else {
            0
        };
        if total > 0 {
            let pct = (received as u64 * 100 / total as u64) as u32;
            print!("\r  Download: {} MB / {} MB  ({}%)  {} kbit/s   ",
                mb, total / (1024 * 1024), pct, speed_kbps);
        } else {
            print!("\r  Download: {} MB  {} kbit/s   ", mb, speed_kbps);
        }
    }
}

// ── Latency measurement ────────────────────────────────────────────────────

fn measure_latency(host: &str) -> Option<u32> {
    let mut ip = [0u8; 4];
    if net::dns(host, &mut ip) != 0 {
        return None;
    }
    println!(
        "  Server: {}.{}.{}.{} ({})",
        ip[0], ip[1], ip[2], ip[3], host
    );

    // Measure TCP handshake time as latency proxy
    let t0 = sys::uptime_ms();
    let sock = net::tcp_connect(&ip, 80, 10000);
    if sock == u32::MAX {
        return None;
    }
    let latency = sys::uptime_ms().wrapping_sub(t0);
    net::tcp_close(sock);
    Some(latency)
}

/// Extract hostname from URL
fn extract_host(url: &str) -> &str {
    let rest = if url.len() > 8 && (url.as_bytes()[4] == b's' || url.as_bytes()[4] == b'S') {
        &url[8..] // https://
    } else if url.len() > 7 {
        &url[7..] // http://
    } else {
        url
    };
    match rest.find('/') {
        Some(i) => {
            let hp = &rest[..i];
            match hp.find(':') {
                Some(j) => &hp[..j],
                None => hp,
            }
        }
        None => match rest.find(':') {
            Some(j) => &rest[..j],
            None => rest,
        },
    }
}

// ── Analysis ───────────────────────────────────────────────────────────────

fn analyze(total_bytes: u32, total_ms: u32, samples: &[ChunkSample]) {
    println!();
    println!("=== Ergebnisse ===");
    println!();

    // Overall speed
    let speed_kbps = if total_ms > 0 {
        (total_bytes as u64 * 8 * 1000) / (total_ms as u64 * 1024)
    } else {
        0
    };
    let speed_mbps_x10 = if total_ms > 0 {
        (total_bytes as u64 * 8 * 1000 * 10) / (total_ms as u64 * 1024 * 1024)
    } else {
        0
    };

    println!(
        "  Durchschnitt:  {}.{} Mbit/s  ({} kbit/s)",
        speed_mbps_x10 / 10,
        speed_mbps_x10 % 10,
        speed_kbps
    );
    println!(
        "  Uebertragen:   {} KB in {}.{} s",
        total_bytes / 1024,
        total_ms / 1000,
        (total_ms % 1000) / 100
    );

    if samples.len() < 2 {
        println!("  (Zu wenige Samples fuer Jitter/Stabilitaet)");
        return;
    }

    // Per-chunk speeds in kbit/s
    let mut speeds: Vec<u64> = Vec::new();
    for s in samples.iter() {
        if s.elapsed_ms > 0 {
            let kbps = (s.bytes as u64 * 8 * 1000) / (s.elapsed_ms as u64 * 1024);
            speeds.push(kbps);
        }
    }

    if speeds.len() < 2 {
        return;
    }

    // Min / Max
    let mut min_speed = speeds[0];
    let mut max_speed = speeds[0];
    let mut sum: u64 = 0;
    for &sp in speeds.iter() {
        if sp < min_speed {
            min_speed = sp;
        }
        if sp > max_speed {
            max_speed = sp;
        }
        sum += sp;
    }
    let avg = sum / speeds.len() as u64;

    let min_mbps_x10 = min_speed * 10 / 1024;
    let max_mbps_x10 = max_speed * 10 / 1024;

    println!(
        "  Minimum:       {}.{} Mbit/s",
        min_mbps_x10 / 10,
        min_mbps_x10 % 10
    );
    println!(
        "  Maximum:       {}.{} Mbit/s",
        max_mbps_x10 / 10,
        max_mbps_x10 % 10
    );

    // Jitter: average absolute deviation between consecutive chunk speeds
    let mut jitter_sum: u64 = 0;
    for i in 1..speeds.len() {
        let diff = if speeds[i] > speeds[i - 1] {
            speeds[i] - speeds[i - 1]
        } else {
            speeds[i - 1] - speeds[i]
        };
        jitter_sum += diff;
    }
    let jitter_kbps = jitter_sum / (speeds.len() as u64 - 1);
    let jitter_mbps_x10 = jitter_kbps * 10 / 1024;

    println!(
        "  Jitter:        {}.{} Mbit/s",
        jitter_mbps_x10 / 10,
        jitter_mbps_x10 % 10
    );

    // Standard deviation for stability
    let mut var_sum: u64 = 0;
    for &sp in speeds.iter() {
        let diff = if sp > avg { sp - avg } else { avg - sp };
        var_sum += diff * diff;
    }
    let variance = var_sum / speeds.len() as u64;
    let stddev = isqrt(variance);
    let stddev_mbps_x10 = stddev * 10 / 1024;

    // Coefficient of variation (CV) as stability indicator
    let cv_x10 = if avg > 0 { stddev * 1000 / avg } else { 0 };

    println!(
        "  Stddev:        {}.{} Mbit/s",
        stddev_mbps_x10 / 10,
        stddev_mbps_x10 % 10
    );

    println!();

    // Stability rating
    let rating = if cv_x10 < 50 {
        "Ausgezeichnet"
    } else if cv_x10 < 100 {
        "Gut"
    } else if cv_x10 < 200 {
        "Akzeptabel"
    } else if cv_x10 < 350 {
        "Instabil"
    } else {
        "Sehr instabil"
    };

    println!(
        "  Stabilitaet:   {} (CV={}.{}%)",
        rating,
        cv_x10 / 10,
        cv_x10 % 10
    );

    // Driver health assessment
    println!();
    println!("=== Treiberbewertung ===");
    println!();

    let drop_threshold = avg / 2;
    let mut drops = 0u32;
    for &sp in speeds.iter() {
        if sp < drop_threshold {
            drops += 1;
        }
    }

    let mut stalls = 0u32;
    for s in samples.iter() {
        if s.elapsed_ms > 5000 {
            stalls += 1;
        }
    }

    if drops == 0 && stalls == 0 && cv_x10 < 150 {
        println!("  Netzwerktreiber: Stabil");
        println!("  Keine Geschwindigkeitseinbrueche oder Stalls erkannt.");
    } else {
        if drops > 0 {
            println!(
                "  Geschwindigkeitseinbrueche: {} von {} Samples unter 50% des Durchschnitts",
                drops,
                speeds.len()
            );
        }
        if stalls > 0 {
            println!(
                "  Stalls: {} Chunks mit >5s Uebertragungszeit (moeglicher Treiber-Stall)",
                stalls
            );
        }
        if cv_x10 >= 150 {
            println!(
                "  Hohe Varianz: Treiber liefert ungleichmaessige Durchsatzraten."
            );
        }
        if drops > speeds.len() as u32 / 4 || stalls > 2 {
            println!("  Bewertung: Treiber moeglicherweise fehlerhaft oder ueberlastet.");
        } else {
            println!("  Bewertung: Leichte Unregelmaessigkeiten, insgesamt funktional.");
        }
    }

    // Per-chunk detail (max 20 samples shown)
    println!();
    println!("=== Chunk-Details (je ~512 KB) ===");
    println!();
    let show = if speeds.len() > 20 { 20 } else { speeds.len() };
    for i in 0..show {
        let sp_mbps_x10 = speeds[i] * 10 / 1024;
        let bar_len = (speeds[i] * 30 / max_speed.max(1)) as usize;
        let bar_len = if bar_len > 30 { 30 } else { bar_len };
        print!(
            "  {:>3}: {}.{} Mbit/s  ",
            i + 1,
            sp_mbps_x10 / 10,
            sp_mbps_x10 % 10
        );
        for _ in 0..bar_len {
            print!("#");
        }
        println!();
    }
    if speeds.len() > 20 {
        println!("  ... ({} weitere Samples)", speeds.len() - 20);
    }
}

/// Integer square root (Newton's method)
fn isqrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

fn print_usage() {
    println!("speedtest {} — Netzwerk-Geschwindigkeitstest fuer anyOS", VERSION);
    println!();
    println!("Verwendung: speedtest [URL]");
    println!();
    println!("Ohne URL wird Tele2 10 MB Testdatei verwendet.");
    println!();
    println!("Optionen:");
    println!("  -b, --big     100 MB Testdatei (genauere Messung)");
    println!("  -h, --help    Hilfe anzeigen");
    println!();
    println!("Misst Download-Geschwindigkeit, Jitter und Treiberstabilitaet");
    println!("durch Herunterladen einer grossen Datei und Analyse der");
    println!("Chunk-Geschwindigkeiten ueber die gesamte Uebertragung.");
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let parsed = args::parse(raw, b"");

    if parsed.has(b'h') || raw.contains("--help") {
        print_usage();
        return;
    }

    let big = parsed.has(b'b') || raw.contains("--big");

    let url_str = if big {
        URL_100MB
    } else if let Some(u) = parsed.pos(0) {
        u
    } else {
        URL_10MB
    };

    println!();
    println!("anyOS Speedtest v{}", VERSION);
    println!("====================");
    println!();
    println!("  URL: {}", url_str);

    // Latency measurement via TCP handshake
    let host = extract_host(url_str);
    match measure_latency(host) {
        Some(lat) => println!("  Latenz (TCP-Handshake): {} ms", lat),
        None => {
            println!("speedtest: DNS-Aufloesung oder Verbindung fehlgeschlagen fuer {}", host);
            return;
        }
    }

    // Init libhttp
    if !libhttp_client::init() {
        println!("speedtest: libhttp.so konnte nicht geladen werden");
        return;
    }

    // Initialize global state
    let now = sys::uptime_ms();
    unsafe {
        STATE = Some(SpeedState {
            start_ms: now,
            chunk_start_bytes: 0,
            chunk_start_ms: now,
            samples: Vec::new(),
            last_progress_mb: 0,
            total_size: 0,
            received: 0,
        });
    }

    println!("  Starte Download...");

    // Drain via libhttp with progress tracking. A speed test must not include
    // filesystem write latency in the measured receive path.
    let received = libhttp_client::drain_progress(
        url_str,
        progress_callback,
        0,
    );

    let end_ms = sys::uptime_ms();
    println!(); // newline after \r progress

    if received.is_none() {
        let err = libhttp_client::last_error();
        let status = libhttp_client::last_status();
        let err_msg = match err {
            1 => "ungueltige URL",
            2 => "DNS-Aufloesung fehlgeschlagen",
            3 => "Verbindung fehlgeschlagen",
            4 => "Senden fehlgeschlagen",
            5 => "Keine Antwort / Timeout",
            6 => "Zu viele Redirects",
            7 => "TLS-Handshake fehlgeschlagen",
            9 => "Dateischreibfehler",
            _ => "unbekannter Fehler",
        };
        if status > 0 {
            println!("speedtest: Fehler: HTTP {} ({})", status, err_msg);
        } else {
            println!("speedtest: Fehler: {}", err_msg);
        }
        return;
    }

    let s = state();
    let total_ms = end_ms.wrapping_sub(s.start_ms);
    let total_bytes = received.unwrap_or(s.received);

    println!("  Empfangen: {} KB", total_bytes / 1024);

    // Flush final partial chunk
    if total_bytes > s.chunk_start_bytes {
        let remaining = total_bytes - s.chunk_start_bytes;
        if remaining > 0 {
            let elapsed = end_ms.wrapping_sub(s.chunk_start_ms);
            if elapsed > 0 && s.samples.len() < MAX_SAMPLES {
                s.samples.push(ChunkSample {
                    bytes: remaining,
                    elapsed_ms: elapsed,
                });
            }
        }
    }

    if total_bytes < 10240 {
        println!("speedtest: Nur {} Bytes empfangen — Server liefert keine Testdatei.", total_bytes);
        println!("  Versuche eine andere URL, z.B.:");
        println!("    speedtest http://proof.ovh.net/files/10Mb.dat");
        println!("    speedtest http://speedtest.tele2.net/10MB.zip");
        return;
    }

    // Move samples out for analysis
    let samples: Vec<ChunkSample> = core::mem::take(&mut s.samples);
    analyze(total_bytes, total_ms, &samples);

    println!();
}
