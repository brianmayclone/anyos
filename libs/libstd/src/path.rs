//! std::path compatible path types.
//!
//! Provides Path, PathBuf, and Components for anyOS.
//! anyOS uses forward-slash paths only (no Windows prefixes/drive letters).

use alloc::borrow::{Cow, ToOwned};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::Deref;

// ── Path ────────────────────────────────────────────────────────────────────

/// A borrowed path slice, like std::path::Path.
///
/// On anyOS this is essentially a &str wrapper since all paths are UTF-8.
#[repr(transparent)]
pub struct Path {
    inner: str,
}

impl Path {
    /// Wrap a string slice as a Path.
    pub fn new<S: AsRef<str> + ?Sized>(s: &S) -> &Path {
        unsafe { &*(s.as_ref() as *const str as *const Path) }
    }

    /// Get the underlying string.
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Get as OS string (on anyOS, same as str).
    pub fn as_os_str(&self) -> &OsStr {
        OsStr::new(&self.inner)
    }

    /// Convert to an owned PathBuf.
    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf {
            inner: String::from(&self.inner),
        }
    }

    /// Check if the path is absolute.
    pub fn is_absolute(&self) -> bool {
        self.inner.starts_with('/')
    }

    /// Check if the path is relative.
    pub fn is_relative(&self) -> bool {
        !self.is_absolute()
    }

    /// Get the parent directory.
    pub fn parent(&self) -> Option<&Path> {
        let s = self.inner.trim_end_matches('/');
        if s.is_empty() {
            return None;
        }
        match s.rfind('/') {
            Some(0) => Some(Path::new("/")),
            Some(i) => Some(Path::new(&self.inner[..i])),
            None => {
                if s == "." || s == ".." {
                    None
                } else {
                    Some(Path::new(""))
                }
            }
        }
    }

    /// Get the file name component.
    pub fn file_name(&self) -> Option<&str> {
        let s = self.inner.trim_end_matches('/');
        if s.is_empty() {
            return None;
        }
        match s.rfind('/') {
            Some(i) => {
                let name = &s[i + 1..];
                if name.is_empty() {
                    None
                } else {
                    Some(name)
                }
            }
            None => Some(s),
        }
    }

    /// Get the file stem (name without extension).
    pub fn file_stem(&self) -> Option<&str> {
        let name = self.file_name()?;
        match name.rfind('.') {
            Some(0) | None => Some(name),
            Some(i) => Some(&name[..i]),
        }
    }

    /// Get the file extension (without dot).
    pub fn extension(&self) -> Option<&str> {
        let name = self.file_name()?;
        match name.rfind('.') {
            Some(0) | None => None,
            Some(i) => Some(&name[i + 1..]),
        }
    }

    /// Join this path with another.
    pub fn join<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        let other = path.as_ref();
        if other.is_absolute() {
            return other.to_path_buf();
        }
        let mut buf = self.to_path_buf();
        buf.push(other);
        buf
    }

    /// Get the path with a new extension.
    pub fn with_extension(&self, ext: &str) -> PathBuf {
        let mut buf = match self.file_stem() {
            Some(_stem) => {
                // Get everything before the last dot in the filename
                let parent = self.parent();
                let stem = self.file_stem().unwrap_or("");
                match parent {
                    Some(p) if !p.as_str().is_empty() => {
                        let mut pb = p.to_path_buf();
                        pb.push(Path::new(stem));
                        pb
                    }
                    _ => PathBuf::from(stem),
                }
            }
            None => self.to_path_buf(),
        };
        if !ext.is_empty() {
            buf.inner.push('.');
            buf.inner.push_str(ext);
        }
        buf
    }

    /// Get the path with a new file name.
    pub fn with_file_name(&self, file_name: &str) -> PathBuf {
        match self.parent() {
            Some(p) => {
                let mut buf = p.to_path_buf();
                buf.push(Path::new(file_name));
                buf
            }
            None => PathBuf::from(file_name),
        }
    }

    /// Check if the path starts with a prefix.
    pub fn starts_with<P: AsRef<Path>>(&self, base: P) -> bool {
        let base_str = base.as_ref().as_str();
        if self.inner == *base_str {
            return true;
        }
        self.inner.starts_with(base_str)
            && (base_str.ends_with('/') || self.inner.as_bytes().get(base_str.len()) == Some(&b'/'))
    }

    /// Check if the path ends with a suffix.
    pub fn ends_with<P: AsRef<Path>>(&self, child: P) -> bool {
        let child_str = child.as_ref().as_str();
        if self.inner == *child_str {
            return true;
        }
        self.inner.ends_with(child_str)
            && (child_str.starts_with('/')
                || self
                    .inner
                    .as_bytes()
                    .get(self.inner.len() - child_str.len() - 1)
                    == Some(&b'/'))
    }

    /// Iterate over path components.
    pub fn components(&self) -> Components<'_> {
        Components {
            path: &self.inner,
            front: 0,
        }
    }

    /// Iterate path ancestors (self, parent, grandparent, ...).
    pub fn ancestors(&self) -> Ancestors<'_> {
        Ancestors { next: Some(self) }
    }

    /// Convert to a string slice (always succeeds on anyOS).
    pub fn to_str(&self) -> Option<&str> {
        Some(&self.inner)
    }

    /// Display the path.
    pub fn display(&self) -> Display<'_> {
        Display { path: self }
    }

    /// Check if this path exists (via stat).
    pub fn exists(&self) -> bool {
        let mut buf = [0u32; 7];
        anyos_std::fs::stat(&self.inner, &mut buf) != u32::MAX
    }

    /// Check if this is a file.
    pub fn is_file(&self) -> bool {
        let mut buf = [0u32; 7];
        if anyos_std::fs::stat(&self.inner, &mut buf) == u32::MAX {
            return false;
        }
        buf[0] == 0 // type 0 = file
    }

    /// Check if this is a directory.
    pub fn is_dir(&self) -> bool {
        let mut buf = [0u32; 7];
        if anyos_std::fs::stat(&self.inner, &mut buf) == u32::MAX {
            return false;
        }
        buf[0] == 1 // type 1 = directory
    }

    /// Strip a prefix from this path.
    pub fn strip_prefix<P: AsRef<Path>>(&self, base: P) -> Result<&Path, StripPrefixError> {
        let base_str = base.as_ref().as_str();
        if self.inner == *base_str {
            return Ok(Path::new(""));
        }
        if self.inner.starts_with(base_str) {
            let rest = &self.inner[base_str.len()..];
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            Ok(Path::new(rest))
        } else {
            Err(StripPrefixError(()))
        }
    }
}

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\"", &self.inner)
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inner)
    }
}

impl AsRef<Path> for Path {
    fn as_ref(&self) -> &Path {
        self
    }
}

impl AsRef<str> for Path {
    fn as_ref(&self) -> &str {
        &self.inner
    }
}

impl AsRef<Path> for str {
    fn as_ref(&self) -> &Path {
        Path::new(self)
    }
}

impl AsRef<Path> for String {
    fn as_ref(&self) -> &Path {
        Path::new(self.as_str())
    }
}

impl ToOwned for Path {
    type Owned = PathBuf;
    fn to_owned(&self) -> PathBuf {
        self.to_path_buf()
    }
}

impl PartialEq for Path {
    fn eq(&self, other: &Path) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Path {}

impl Hash for Path {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

impl PartialOrd for Path {
    fn partial_cmp(&self, other: &Path) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Path {
    fn cmp(&self, other: &Path) -> core::cmp::Ordering {
        self.inner.cmp(&other.inner)
    }
}

// ── PathBuf ─────────────────────────────────────────────────────────────────

/// An owned path, like std::path::PathBuf.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PathBuf {
    inner: String,
}

impl PathBuf {
    pub fn new() -> Self {
        PathBuf {
            inner: String::new(),
        }
    }

    pub fn from<S: Into<String>>(s: S) -> Self {
        PathBuf { inner: s.into() }
    }

    pub fn push<P: AsRef<Path>>(&mut self, path: P) {
        let path = path.as_ref().as_str();
        if path.starts_with('/') {
            self.inner.clear();
            self.inner.push_str(path);
            return;
        }
        if !self.inner.is_empty() && !self.inner.ends_with('/') {
            self.inner.push('/');
        }
        self.inner.push_str(path);
    }

    pub fn pop(&mut self) -> bool {
        match self.parent().map(|p| p.as_str().len()) {
            Some(len) => {
                self.inner.truncate(len);
                true
            }
            None => false,
        }
    }

    pub fn set_file_name(&mut self, file_name: &str) {
        if self.file_name().is_some() {
            self.pop();
        }
        self.push(Path::new(file_name));
    }

    pub fn set_extension(&mut self, ext: &str) -> bool {
        let stem = match self.file_stem() {
            Some(s) => String::from(s),
            None => return false,
        };
        let new = if ext.is_empty() {
            stem
        } else {
            let mut s = stem;
            s.push('.');
            s.push_str(ext);
            s
        };
        self.set_file_name(&new);
        true
    }

    pub fn as_path(&self) -> &Path {
        Path::new(&self.inner)
    }

    pub fn into_string(self) -> String {
        self.inner
    }

    pub fn as_os_str(&self) -> &OsStr {
        OsStr::new(&self.inner)
    }

    pub fn into_os_string(self) -> OsString {
        OsString { inner: self.inner }
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }
}

impl Default for PathBuf {
    fn default() -> Self {
        PathBuf::new()
    }
}

impl Deref for PathBuf {
    type Target = Path;
    fn deref(&self) -> &Path {
        Path::new(&self.inner)
    }
}

impl AsRef<Path> for PathBuf {
    fn as_ref(&self) -> &Path {
        Path::new(&self.inner)
    }
}

impl AsRef<str> for PathBuf {
    fn as_ref(&self) -> &str {
        &self.inner
    }
}

impl From<String> for PathBuf {
    fn from(s: String) -> Self {
        PathBuf { inner: s }
    }
}

impl From<&str> for PathBuf {
    fn from(s: &str) -> Self {
        PathBuf {
            inner: String::from(s),
        }
    }
}

impl From<&Path> for PathBuf {
    fn from(p: &Path) -> Self {
        p.to_path_buf()
    }
}

impl<'a> From<Cow<'a, Path>> for PathBuf {
    fn from(cow: Cow<'a, Path>) -> Self {
        match cow {
            Cow::Borrowed(p) => p.to_path_buf(),
            Cow::Owned(p) => p,
        }
    }
}

impl fmt::Debug for PathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\"", &self.inner)
    }
}

impl fmt::Display for PathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inner)
    }
}

impl core::borrow::Borrow<Path> for PathBuf {
    fn borrow(&self) -> &Path {
        self.deref()
    }
}

// ── OsStr / OsString ───────────────────────────────────────────────────────

/// OS string slice. On anyOS, this is always valid UTF-8.
#[repr(transparent)]
pub struct OsStr {
    inner: str,
}

impl OsStr {
    pub fn new<S: AsRef<str> + ?Sized>(s: &S) -> &OsStr {
        unsafe { &*(s.as_ref() as *const str as *const OsStr) }
    }

    pub fn to_str(&self) -> Option<&str> {
        Some(&self.inner)
    }

    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.inner)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn to_os_string(&self) -> OsString {
        OsString {
            inner: String::from(&self.inner),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }
}

impl AsRef<str> for OsStr {
    fn as_ref(&self) -> &str {
        &self.inner
    }
}

impl AsRef<Path> for OsStr {
    fn as_ref(&self) -> &Path {
        Path::new(&self.inner)
    }
}

impl AsRef<OsStr> for str {
    fn as_ref(&self) -> &OsStr {
        OsStr::new(self)
    }
}

impl AsRef<OsStr> for String {
    fn as_ref(&self) -> &OsStr {
        OsStr::new(self.as_str())
    }
}

impl AsRef<OsStr> for Path {
    fn as_ref(&self) -> &OsStr {
        OsStr::new(&self.inner)
    }
}

impl AsRef<OsStr> for PathBuf {
    fn as_ref(&self) -> &OsStr {
        OsStr::new(&self.inner)
    }
}

impl ToOwned for OsStr {
    type Owned = OsString;
    fn to_owned(&self) -> OsString {
        self.to_os_string()
    }
}

impl core::borrow::Borrow<OsStr> for OsString {
    fn borrow(&self) -> &OsStr {
        OsStr::new(&self.inner)
    }
}

impl PartialEq for OsStr {
    fn eq(&self, other: &OsStr) -> bool {
        self.inner == other.inner
    }
}

impl Eq for OsStr {}

impl Hash for OsStr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

impl fmt::Debug for OsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\"", &self.inner)
    }
}

impl fmt::Display for OsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inner)
    }
}

impl PartialEq<str> for OsStr {
    fn eq(&self, other: &str) -> bool {
        self.inner == *other
    }
}

/// Owned OS string. On anyOS, always valid UTF-8.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct OsString {
    inner: String,
}

impl OsString {
    pub fn new() -> Self {
        OsString {
            inner: String::new(),
        }
    }

    pub fn from<S: Into<String>>(s: S) -> Self {
        OsString { inner: s.into() }
    }

    pub fn as_os_str(&self) -> &OsStr {
        OsStr::new(&self.inner)
    }

    pub fn into_string(self) -> Result<String, OsString> {
        Ok(self.inner) // Always valid UTF-8 on anyOS
    }

    pub fn push<S: AsRef<OsStr>>(&mut self, s: S) {
        self.inner.push_str(s.as_ref().as_ref());
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl Default for OsString {
    fn default() -> Self {
        OsString::new()
    }
}

impl Deref for OsString {
    type Target = OsStr;
    fn deref(&self) -> &OsStr {
        OsStr::new(&self.inner)
    }
}

impl AsRef<OsStr> for OsString {
    fn as_ref(&self) -> &OsStr {
        OsStr::new(&self.inner)
    }
}

impl AsRef<Path> for OsString {
    fn as_ref(&self) -> &Path {
        Path::new(&self.inner)
    }
}

impl From<String> for OsString {
    fn from(s: String) -> Self {
        OsString { inner: s }
    }
}

impl From<&str> for OsString {
    fn from(s: &str) -> Self {
        OsString {
            inner: String::from(s),
        }
    }
}

impl From<PathBuf> for OsString {
    fn from(p: PathBuf) -> Self {
        OsString { inner: p.inner }
    }
}

impl From<OsString> for PathBuf {
    fn from(s: OsString) -> Self {
        PathBuf { inner: s.inner }
    }
}

impl fmt::Display for OsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.inner)
    }
}

// ── Components ──────────────────────────────────────────────────────────────

/// A path component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Component<'a> {
    /// The root directory `/`.
    RootDir,
    /// Current directory `.`.
    CurDir,
    /// Parent directory `..`.
    ParentDir,
    /// A normal path component.
    Normal(&'a str),
}

impl<'a> Component<'a> {
    pub fn as_os_str(&self) -> &OsStr {
        match self {
            Component::RootDir => OsStr::new("/"),
            Component::CurDir => OsStr::new("."),
            Component::ParentDir => OsStr::new(".."),
            Component::Normal(s) => OsStr::new(s),
        }
    }
}

impl<'a> AsRef<Path> for Component<'a> {
    fn as_ref(&self) -> &Path {
        match self {
            Component::RootDir => Path::new("/"),
            Component::CurDir => Path::new("."),
            Component::ParentDir => Path::new(".."),
            Component::Normal(s) => Path::new(s),
        }
    }
}

/// Iterator over path components.
pub struct Components<'a> {
    path: &'a str,
    front: usize,
}

impl<'a> Iterator for Components<'a> {
    type Item = Component<'a>;

    fn next(&mut self) -> Option<Component<'a>> {
        if self.front >= self.path.len() {
            return None;
        }

        let remaining = &self.path[self.front..];

        // Root at the start
        if self.front == 0 && remaining.starts_with('/') {
            self.front = 1;
            // Skip consecutive slashes
            while self.front < self.path.len() && self.path.as_bytes()[self.front] == b'/' {
                self.front += 1;
            }
            return Some(Component::RootDir);
        }

        // Skip slashes
        let start = remaining.find(|c: char| c != '/')?;
        let abs_start = self.front + start;

        // Find end of component
        let rest = &self.path[abs_start..];
        let end = rest.find('/').unwrap_or(rest.len());
        let component = &self.path[abs_start..abs_start + end];
        self.front = abs_start + end;

        match component {
            "." => Some(Component::CurDir),
            ".." => Some(Component::ParentDir),
            s => Some(Component::Normal(s)),
        }
    }
}

impl<'a> Components<'a> {
    pub fn as_path(&self) -> &'a Path {
        Path::new(&self.path[self.front..])
    }
}

// ── Ancestors ───────────────────────────────────────────────────────────────

/// Iterator over path ancestors.
pub struct Ancestors<'a> {
    next: Option<&'a Path>,
}

impl<'a> Iterator for Ancestors<'a> {
    type Item = &'a Path;

    fn next(&mut self) -> Option<&'a Path> {
        let current = self.next?;
        self.next = current.parent();
        Some(current)
    }
}

// ── Display ─────────────────────────────────────────────────────────────────

pub struct Display<'a> {
    path: &'a Path,
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.path.as_str())
    }
}

// ── StripPrefixError ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripPrefixError(());

impl fmt::Display for StripPrefixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("prefix not found")
    }
}

// ── Convenience re-exports ──────────────────────────────────────────────────

/// Check if a path is a separator character.
pub fn is_separator(c: char) -> bool {
    c == '/'
}

/// The main separator character.
pub const MAIN_SEPARATOR: char = '/';

/// The main separator as a string.
pub const MAIN_SEPARATOR_STR: &str = "/";
