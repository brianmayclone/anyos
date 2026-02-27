//! Semantic version parsing and comparison.
//!
//! Parses `"MAJOR.MINOR.PATCH"` strings and provides ordering.
//! Used for dependency resolution and upgrade detection.

use alloc::string::String;
use core::cmp::Ordering;
use core::fmt;

/// A parsed semantic version (MAJOR.MINOR.PATCH).
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// Parse a version string like `"1.2.3"`.
    /// Also handles `"1.2"` (patch=0) and `"1"` (minor=0, patch=0).
    pub fn parse(s: &str) -> Option<Version> {
        let mut parts = s.split('.');
        let major = parse_u32(parts.next()?)?;
        let minor = parts.next().map(parse_u32).unwrap_or(Some(0))?;
        let patch = parts.next().map(parse_u32).unwrap_or(Some(0))?;
        // Reject extra components
        if parts.next().is_some() {
            return None;
        }
        Some(Version { major, minor, patch })
    }

    /// Format as `"MAJOR.MINOR.PATCH"`.
    pub fn to_string(&self) -> String {
        alloc::format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A version constraint like `">=1.0.0"` or `"<2.0.0"`.
#[derive(Debug, Clone)]
pub struct VersionConstraint {
    pub op: ConstraintOp,
    pub version: Version,
}

/// Comparison operator for version constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintOp {
    /// `>=`
    Gte,
    /// `>`
    Gt,
    /// `=` or `==`
    Eq,
    /// `<`
    Lt,
    /// `<=`
    Lte,
}

impl VersionConstraint {
    /// Check if a version satisfies this constraint.
    pub fn satisfied_by(&self, v: &Version) -> bool {
        match self.op {
            ConstraintOp::Gte => v >= &self.version,
            ConstraintOp::Gt => v > &self.version,
            ConstraintOp::Eq => v == &self.version,
            ConstraintOp::Lt => v < &self.version,
            ConstraintOp::Lte => v <= &self.version,
        }
    }
}

/// Parse a dependency string like `"libfoo>=1.0.0"` or `"bar"` (any version).
/// Returns `(package_name, optional_constraint)`.
pub fn parse_dependency(dep: &str) -> (&str, Option<VersionConstraint>) {
    // Try each operator in order (longest first to avoid ambiguity)
    for (op_str, op) in &[
        (">=", ConstraintOp::Gte),
        ("<=", ConstraintOp::Lte),
        ("==", ConstraintOp::Eq),
        (">", ConstraintOp::Gt),
        ("<", ConstraintOp::Lt),
        ("=", ConstraintOp::Eq),
    ] {
        if let Some(pos) = dep.find(op_str) {
            let name = &dep[..pos];
            let ver_str = &dep[pos + op_str.len()..];
            if let Some(version) = Version::parse(ver_str) {
                return (name, Some(VersionConstraint { op: *op, version }));
            }
        }
    }
    // No constraint — any version
    (dep, None)
}

/// Parse a u32 from a decimal string.
fn parse_u32(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut n: u32 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}
