//! SemVer version parsing and requirement matching.
//!
//! Implements Cargo-compatible version requirements:
//! - `^1.2.3` (caret, default) — >=1.2.3, <2.0.0
//! - `~1.2.3` (tilde) — >=1.2.3, <1.3.0
//! - `=1.2.3` (exact)
//! - `>=1.0`, `<2.0`, `<=1.5`, `>0.9`
//! - `*` (any)
//! - Comma-separated compound: `>=1.0, <2.0`

use crate::prelude::*;

/// A parsed semantic version: major.minor.patch[-pre].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub pre: String,
}

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        // Split off pre-release tag
        let (version_part, pre) = if let Some(pos) = s.find('-') {
            (&s[..pos], s[pos + 1..].to_string())
        } else {
            (s, String::new())
        };

        let parts: Vec<&str> = version_part.split('.').collect();
        let major = parts.first()?.parse::<u32>().ok()?;
        let minor = parts.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        let patch = parts.get(2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);

        Some(Version { major, minor, patch, pre })
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    /// Comparable tuple for ordering.
    fn as_tuple(&self) -> (u32, u32, u32) {
        (self.major, self.minor, self.patch)
    }

    pub fn to_string(&self) -> String {
        if self.pre.is_empty() {
            format!("{}.{}.{}", self.major, self.minor, self.patch)
        } else {
            format!("{}.{}.{}-{}", self.major, self.minor, self.patch, self.pre)
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_tuple().cmp(&other.as_tuple())
    }
}

/// A single version comparator (e.g. `>=1.0.0` or `<2.0.0`).
#[derive(Debug, Clone)]
enum Comparator {
    /// ^1.2.3 — caret (most common, Cargo default)
    Caret(Version),
    /// ~1.2.3 — tilde
    Tilde(Version),
    /// =1.2.3 — exact
    Exact(Version),
    /// >=, >, <=, <
    Range(RangeOp, Version),
    /// * — any version
    Any,
}

#[derive(Debug, Clone, Copy)]
enum RangeOp {
    Ge, Gt, Le, Lt,
}

impl Comparator {
    fn matches(&self, v: &Version) -> bool {
        match self {
            Comparator::Any => true,
            Comparator::Exact(req) => v.major == req.major && v.minor == req.minor && v.patch == req.patch,
            Comparator::Caret(req) => caret_matches(req, v),
            Comparator::Tilde(req) => tilde_matches(req, v),
            Comparator::Range(op, req) => {
                let vt = v.as_tuple();
                let rt = req.as_tuple();
                match op {
                    RangeOp::Ge => vt >= rt,
                    RangeOp::Gt => vt > rt,
                    RangeOp::Le => vt <= rt,
                    RangeOp::Lt => vt < rt,
                }
            }
        }
    }
}

/// Caret requirement: the leftmost non-zero component must not change.
fn caret_matches(req: &Version, v: &Version) -> bool {
    if v.as_tuple() < req.as_tuple() {
        return false;
    }
    if req.major != 0 {
        // ^X.Y.Z: X must match, Y.Z can increase
        v.major == req.major
    } else if req.minor != 0 {
        // ^0.Y.Z: major=0, Y must match, Z can increase
        v.major == 0 && v.minor == req.minor
    } else {
        // ^0.0.Z: exact match
        v.major == 0 && v.minor == 0 && v.patch == req.patch
    }
}

/// Tilde requirement: minor must match.
fn tilde_matches(req: &Version, v: &Version) -> bool {
    if v.as_tuple() < req.as_tuple() {
        return false;
    }
    v.major == req.major && v.minor == req.minor
}

/// A version requirement that may contain multiple comma-separated comparators.
#[derive(Debug, Clone)]
pub struct VersionReq {
    comparators: Vec<Comparator>,
}

impl VersionReq {
    /// Parse a Cargo-style version requirement string.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() || s == "*" {
            return Some(VersionReq { comparators: vec![Comparator::Any] });
        }

        let mut comparators = Vec::new();
        for part in s.split(',') {
            let part = part.trim();
            if part.is_empty() { continue; }
            comparators.push(parse_single_comparator(part)?);
        }

        if comparators.is_empty() {
            return None;
        }

        Some(VersionReq { comparators })
    }

    /// Check if a version satisfies this requirement.
    pub fn matches(&self, v: &Version) -> bool {
        // Skip pre-release versions unless explicitly requested
        if v.is_prerelease() {
            let any_prerelease_req = self.comparators.iter().any(|c| {
                match c {
                    Comparator::Exact(r) | Comparator::Caret(r) | Comparator::Tilde(r) => r.is_prerelease(),
                    Comparator::Range(_, r) => r.is_prerelease(),
                    Comparator::Any => false,
                }
            });
            if !any_prerelease_req {
                return false;
            }
        }
        self.comparators.iter().all(|c| c.matches(v))
    }
}

fn parse_single_comparator(s: &str) -> Option<Comparator> {
    let s = s.trim();
    if s == "*" {
        return Some(Comparator::Any);
    }
    if let Some(rest) = s.strip_prefix("^") {
        return Some(Comparator::Caret(Version::parse(rest)?));
    }
    if let Some(rest) = s.strip_prefix("~") {
        return Some(Comparator::Tilde(Version::parse(rest)?));
    }
    if let Some(rest) = s.strip_prefix(">=") {
        return Some(Comparator::Range(RangeOp::Ge, Version::parse(rest)?));
    }
    if let Some(rest) = s.strip_prefix(">") {
        return Some(Comparator::Range(RangeOp::Gt, Version::parse(rest)?));
    }
    if let Some(rest) = s.strip_prefix("<=") {
        return Some(Comparator::Range(RangeOp::Le, Version::parse(rest)?));
    }
    if let Some(rest) = s.strip_prefix("<") {
        return Some(Comparator::Range(RangeOp::Lt, Version::parse(rest)?));
    }
    if let Some(rest) = s.strip_prefix("=") {
        return Some(Comparator::Exact(Version::parse(rest)?));
    }

    // Bare version: treat as caret (Cargo default)
    // "1.0" → "^1.0.0", "1.2.3" → "^1.2.3"
    Some(Comparator::Caret(Version::parse(s)?))
}

/// Given a list of available versions and a requirement, find the best match.
/// Returns the newest non-yanked version that satisfies the requirement.
pub fn resolve_best(
    versions: &[(Version, bool)],  // (version, yanked)
    req: &VersionReq,
) -> Option<Version> {
    let mut best: Option<&Version> = None;
    for (v, yanked) in versions {
        if *yanked { continue; }
        if !req.matches(v) { continue; }
        match best {
            None => best = Some(v),
            Some(current) => {
                if v > current {
                    best = Some(v);
                }
            }
        }
    }
    best.cloned()
}
