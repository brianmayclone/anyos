//! std::time compatible time types.

use core::ops::{Add, Sub};
pub use core::time::Duration;

/// A point in time, mirroring std::time::Instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant {
    ms: u64,
}

impl Instant {
    /// Get the current instant.
    pub fn now() -> Self {
        Instant { ms: anyos_std::sys::uptime_ms() as u64 }
    }

    /// Duration since an earlier instant.
    pub fn duration_since(&self, earlier: Instant) -> Duration {
        Duration::from_millis(self.ms.saturating_sub(earlier.ms))
    }

    /// Duration elapsed since this instant.
    pub fn elapsed(&self) -> Duration {
        Self::now().duration_since(*self)
    }

    /// Checked addition.
    pub fn checked_add(&self, duration: Duration) -> Option<Instant> {
        self.ms.checked_add(duration.as_millis() as u64).map(|ms| Instant { ms })
    }

    /// Checked subtraction.
    pub fn checked_sub(&self, duration: Duration) -> Option<Instant> {
        self.ms.checked_sub(duration.as_millis() as u64).map(|ms| Instant { ms })
    }

    /// Checked duration since another instant.
    pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
        self.ms.checked_sub(earlier.ms).map(Duration::from_millis)
    }

    /// Saturating duration since another instant.
    pub fn saturating_duration_since(&self, earlier: Instant) -> Duration {
        self.checked_duration_since(earlier).unwrap_or_default()
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;
    fn add(self, rhs: Duration) -> Instant {
        Instant { ms: self.ms + rhs.as_millis() as u64 }
    }
}

impl Sub<Duration> for Instant {
    type Output = Instant;
    fn sub(self, rhs: Duration) -> Instant {
        Instant { ms: self.ms - rhs.as_millis() as u64 }
    }
}

impl Sub<Instant> for Instant {
    type Output = Duration;
    fn sub(self, rhs: Instant) -> Duration {
        self.duration_since(rhs)
    }
}

/// A point in time relative to the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SystemTime {
    pub(crate) secs: u64,
}

impl SystemTime {
    pub const UNIX_EPOCH: SystemTime = SystemTime { secs: 0 };

    /// Get the current system time.
    pub fn now() -> Self {
        let mut buf = [0u8; 8];
        anyos_std::sys::time(&mut buf);
        // buf = [year_lo, year_hi, month, day, hour, min, sec, 0]
        let year = buf[0] as u32 | ((buf[1] as u32) << 8);
        let month = buf[2] as u32;
        let day = buf[3] as u32;
        let hour = buf[4] as u32;
        let min = buf[5] as u32;
        let sec = buf[6] as u32;

        // Approximate seconds since epoch
        let days = approximate_days(year, month, day);
        let secs = (days as u64) * 86400 + (hour as u64) * 3600 + (min as u64) * 60 + sec as u64;
        SystemTime { secs }
    }

    /// Duration since an earlier time.
    pub fn duration_since(&self, earlier: SystemTime) -> Result<Duration, SystemTimeError> {
        if self.secs >= earlier.secs {
            Ok(Duration::from_secs(self.secs - earlier.secs))
        } else {
            Err(SystemTimeError(Duration::from_secs(earlier.secs - self.secs)))
        }
    }

    /// Duration elapsed since this time.
    pub fn elapsed(&self) -> Result<Duration, SystemTimeError> {
        Self::now().duration_since(*self)
    }

    /// Checked addition.
    pub fn checked_add(&self, duration: Duration) -> Option<SystemTime> {
        self.secs.checked_add(duration.as_secs()).map(|secs| SystemTime { secs })
    }

    /// Checked subtraction.
    pub fn checked_sub(&self, duration: Duration) -> Option<SystemTime> {
        self.secs.checked_sub(duration.as_secs()).map(|secs| SystemTime { secs })
    }
}

impl Add<Duration> for SystemTime {
    type Output = SystemTime;
    fn add(self, rhs: Duration) -> SystemTime {
        SystemTime { secs: self.secs + rhs.as_secs() }
    }
}

impl Sub<Duration> for SystemTime {
    type Output = SystemTime;
    fn sub(self, rhs: Duration) -> SystemTime {
        SystemTime { secs: self.secs - rhs.as_secs() }
    }
}

/// The Unix epoch constant.
pub const UNIX_EPOCH: SystemTime = SystemTime::UNIX_EPOCH;

/// Error from SystemTime::duration_since when the time is earlier.
#[derive(Debug, Clone)]
pub struct SystemTimeError(Duration);

impl SystemTimeError {
    pub fn duration(&self) -> Duration {
        self.0
    }
}

impl core::fmt::Display for SystemTimeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "second time is later than self")
    }
}

/// Approximate days since Unix epoch (Jan 1, 1970).
fn approximate_days(year: u32, month: u32, day: u32) -> u32 {
    let mut y = year as i32;
    let mut m = month as i32;
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468; // days since epoch
    if days < 0 { 0 } else { days as u32 }
}
