//! Zentrale Kernel-Zeitquelle (Wall-Clock).
//!
//! Strategie (x86_64):
//! 1. Beim ersten Aufruf von `wall_clock_unix_secs()` wird einmalig die
//!    CMOS-RTC (MC146818) gelesen und als `BOOT_UNIX_SECS` gespeichert.
//!    Gleichzeitig wird der PIT/TSC-Millisekunden-Stand gespeichert.
//! 2. Alle weiteren Aufrufe rechnen die vergangene Zeit seit dem RTC-Sample
//!    per `real_ms_since_boot()` aus — ohne erneuten CMOS-Poll.
//!
//! Alle Subsysteme (CoreFS, Logging, …) sollen diese API nutzen statt eigene
//! RTC-Initialisierung zu duplizieren.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static BOOT_UNIX_SECS: AtomicU64 = AtomicU64::new(0);
static BOOT_OFFSET_MS: AtomicU64 = AtomicU64::new(0);
static INITIALISED: AtomicBool = AtomicBool::new(false);

/// Konvertiert (Jahr, Monat, Tag, Stunde, Minute, Sekunde) UTC in Unix-Sekunden.
fn ymd_hms_to_unix(year: u16, month: u8, day: u8, hour: u8, min: u8, sec: u8) -> u64 {
    const CUMUL: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let year = if year < 1970 { 1970u32 } else { year as u32 };
    let month = month.clamp(1, 12) as u32;
    let day = day.clamp(1, 31) as u32;

    let mut days: u32 = 0;
    for y in 1970..year {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        days += if leap { 366 } else { 365 };
    }
    days += CUMUL[(month - 1) as usize];
    if month > 2 && ((year % 4 == 0 && year % 100 != 0) || year % 400 == 0) {
        days += 1;
    }
    days += day - 1;

    (days as u64) * 86_400
        + (hour.min(59) as u64) * 3600
        + (min.min(59) as u64) * 60
        + sec.min(59) as u64
}

/// Liefert die aktuelle Unix-Zeit in Sekunden (UTC).
///
/// Auf x86_64: einmaliger RTC-Read beim ersten Aufruf, danach PIT-relativ.
/// Auf anderen Architekturen: Epoch (0), bis ein Plattform-RTC-Pfad existiert.
pub fn wall_clock_unix_secs() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        let ms_now = crate::arch::x86::pit::real_ms_since_boot();
        if !INITIALISED.load(Ordering::Acquire) {
            let t = crate::drivers::rtc::read_time();
            let unix =
                ymd_hms_to_unix(t.year, t.month, t.day, t.hours, t.minutes, t.seconds);
            // Races sind harmlos: letzter Schreiber gewinnt, beide sehen die
            // gleiche RTC-Sekunde (±1).
            BOOT_UNIX_SECS.store(unix, Ordering::Relaxed);
            BOOT_OFFSET_MS.store(ms_now, Ordering::Relaxed);
            INITIALISED.store(true, Ordering::Release);
            return unix;
        }
        let base = BOOT_UNIX_SECS.load(Ordering::Relaxed);
        let off = BOOT_OFFSET_MS.load(Ordering::Relaxed);
        base + ms_now.saturating_sub(off) / 1000
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}
