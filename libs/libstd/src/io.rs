//! std::io compatible I/O traits, types, and helpers.
//!
//! Maps anyos_std::error::Error to std::io::Error and provides
//! Read, Write, Seek traits plus BufReader, BufWriter, and Cursor.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

// ── Error / Result ──────────────────────────────────────────────────────────

/// I/O error kind, mirroring std::io::ErrorKind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    NotFound,
    PermissionDenied,
    ConnectionRefused,
    AlreadyExists,
    WouldBlock,
    InvalidInput,
    InvalidData,
    TimedOut,
    WriteZero,
    Interrupted,
    UnexpectedEof,
    BrokenPipe,
    OutOfMemory,
    NotADirectory,
    IsADirectory,
    Other,
}

/// I/O error type compatible with std::io::Error.
pub struct Error {
    kind: ErrorKind,
    message: Option<Box<str>>,
}

impl Error {
    /// Create a new error from a kind and a message.
    pub fn new<E: Into<Box<str>>>(kind: ErrorKind, error: E) -> Self {
        Error {
            kind,
            message: Some(error.into()),
        }
    }

    /// Create an error from just a kind (no message).
    pub fn from_kind(kind: ErrorKind) -> Self {
        Error {
            kind,
            message: None,
        }
    }

    /// Get the error kind.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Convert from anyos_std error.
    pub fn from_anyos(e: anyos_std::error::Error) -> Self {
        let kind = match e {
            anyos_std::error::Error::NotFound => ErrorKind::NotFound,
            anyos_std::error::Error::PermissionDenied => ErrorKind::PermissionDenied,
            anyos_std::error::Error::ConnectionRefused => ErrorKind::ConnectionRefused,
            anyos_std::error::Error::AlreadyExists => ErrorKind::AlreadyExists,
            anyos_std::error::Error::WouldBlock => ErrorKind::WouldBlock,
            anyos_std::error::Error::InvalidInput => ErrorKind::InvalidInput,
            anyos_std::error::Error::TimedOut => ErrorKind::TimedOut,
            anyos_std::error::Error::BrokenPipe => ErrorKind::BrokenPipe,
            anyos_std::error::Error::OutOfMemory => ErrorKind::OutOfMemory,
            anyos_std::error::Error::NotADirectory => ErrorKind::NotADirectory,
            anyos_std::error::Error::IsADirectory => ErrorKind::IsADirectory,
            anyos_std::error::Error::NoSpace => ErrorKind::Other,
            anyos_std::error::Error::Other(_) => ErrorKind::Other,
        };
        Error {
            kind,
            message: None,
        }
    }

    /// Convert to anyos_std error.
    pub fn to_anyos(&self) -> anyos_std::error::Error {
        match self.kind {
            ErrorKind::NotFound => anyos_std::error::Error::NotFound,
            ErrorKind::PermissionDenied => anyos_std::error::Error::PermissionDenied,
            ErrorKind::ConnectionRefused => anyos_std::error::Error::ConnectionRefused,
            ErrorKind::AlreadyExists => anyos_std::error::Error::AlreadyExists,
            ErrorKind::WouldBlock => anyos_std::error::Error::WouldBlock,
            ErrorKind::InvalidInput => anyos_std::error::Error::InvalidInput,
            ErrorKind::TimedOut => anyos_std::error::Error::TimedOut,
            ErrorKind::BrokenPipe => anyos_std::error::Error::BrokenPipe,
            ErrorKind::OutOfMemory => anyos_std::error::Error::OutOfMemory,
            ErrorKind::NotADirectory => anyos_std::error::Error::NotADirectory,
            ErrorKind::IsADirectory => anyos_std::error::Error::IsADirectory,
            _ => anyos_std::error::Error::Other(0),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Error");
        d.field("kind", &self.kind);
        if let Some(ref msg) = self.message {
            d.field("message", msg);
        }
        d.finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref msg) = self.message {
            write!(f, "{}", msg)
        } else {
            match self.kind {
                ErrorKind::NotFound => write!(f, "entity not found"),
                ErrorKind::PermissionDenied => write!(f, "permission denied"),
                ErrorKind::ConnectionRefused => write!(f, "connection refused"),
                ErrorKind::AlreadyExists => write!(f, "entity already exists"),
                ErrorKind::WouldBlock => write!(f, "operation would block"),
                ErrorKind::InvalidInput => write!(f, "invalid input parameter"),
                ErrorKind::InvalidData => write!(f, "invalid data"),
                ErrorKind::TimedOut => write!(f, "operation timed out"),
                ErrorKind::WriteZero => write!(f, "write zero"),
                ErrorKind::Interrupted => write!(f, "operation interrupted"),
                ErrorKind::UnexpectedEof => write!(f, "unexpected end of file"),
                ErrorKind::BrokenPipe => write!(f, "broken pipe"),
                ErrorKind::OutOfMemory => write!(f, "out of memory"),
                ErrorKind::NotADirectory => write!(f, "not a directory"),
                ErrorKind::IsADirectory => write!(f, "is a directory"),
                ErrorKind::Other => write!(f, "other error"),
            }
        }
    }
}

impl From<anyos_std::error::Error> for Error {
    fn from(e: anyos_std::error::Error) -> Self {
        Error::from_anyos(e)
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Error::from_kind(kind)
    }
}

/// I/O result type alias.
pub type Result<T> = core::result::Result<T, Error>;

// ── Seek ────────────────────────────────────────────────────────────────────

/// Seek position, mirroring std::io::SeekFrom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekFrom {
    Start(u64),
    End(i64),
    Current(i64),
}

/// Trait for seekable I/O, mirroring std::io::Seek.
pub trait Seek {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64>;

    fn stream_position(&mut self) -> Result<u64> {
        self.seek(SeekFrom::Current(0))
    }

    fn rewind(&mut self) -> Result<()> {
        self.seek(SeekFrom::Start(0))?;
        Ok(())
    }
}

// ── Read ────────────────────────────────────────────────────────────────────

/// Trait for readable I/O, mirroring std::io::Read.
pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.read(buf) {
                Ok(0) => return Err(Error::from_kind(ErrorKind::UnexpectedEof)),
                Ok(n) => buf = &mut buf[n..],
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let mut total = 0;
        let mut tmp = [0u8; 4096];
        loop {
            match self.read(&mut tmp) {
                Ok(0) => return Ok(total),
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    total += n;
                }
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    fn read_to_string(&mut self, buf: &mut String) -> Result<usize> {
        let mut bytes = Vec::new();
        let n = self.read_to_end(&mut bytes)?;
        match core::str::from_utf8(&bytes) {
            Ok(s) => {
                buf.push_str(s);
                Ok(n)
            }
            Err(_) => Err(Error::new(
                ErrorKind::InvalidData,
                "stream did not contain valid UTF-8",
            )),
        }
    }

    fn bytes(self) -> Bytes<Self>
    where
        Self: Sized,
    {
        Bytes { inner: self }
    }

    fn chain<R: Read>(self, next: R) -> Chain<Self, R>
    where
        Self: Sized,
    {
        Chain {
            first: self,
            second: next,
            done_first: false,
        }
    }

    fn take(self, limit: u64) -> Take<Self>
    where
        Self: Sized,
    {
        Take { inner: self, limit }
    }
}

// ── Write ───────────────────────────────────────────────────────────────────

/// Trait for writable I/O, mirroring std::io::Write.
pub trait Write {
    fn write(&mut self, buf: &[u8]) -> Result<usize>;

    fn flush(&mut self) -> Result<()>;

    fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.write(buf) {
                Ok(0) => return Err(Error::from_kind(ErrorKind::WriteZero)),
                Ok(n) => buf = &buf[n..],
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn write_fmt(&mut self, fmt: fmt::Arguments<'_>) -> Result<()> {
        // Write formatted string into a buffer first
        struct Adapter<'a, T: ?Sized + 'a> {
            inner: &'a mut T,
            error: Result<()>,
        }

        impl<T: Write + ?Sized> fmt::Write for Adapter<'_, T> {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                match self.inner.write_all(s.as_bytes()) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        self.error = Err(e);
                        Err(fmt::Error)
                    }
                }
            }
        }

        let mut output = Adapter {
            inner: self,
            error: Ok(()),
        };
        match fmt::write(&mut output, fmt) {
            Ok(()) => Ok(()),
            Err(..) => {
                if output.error.is_err() {
                    output.error
                } else {
                    Err(Error::new(ErrorKind::Other, "formatter error"))
                }
            }
        }
    }
}

// ── BufRead ─────────────────────────────────────────────────────────────────

/// Trait for buffered reading, mirroring std::io::BufRead.
pub trait BufRead: Read {
    fn fill_buf(&mut self) -> Result<&[u8]>;
    fn consume(&mut self, amt: usize);

    fn read_until(&mut self, byte: u8, buf: &mut Vec<u8>) -> Result<usize> {
        let mut total = 0;
        loop {
            let available = match self.fill_buf() {
                Ok(b) => b,
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if available.is_empty() {
                return Ok(total);
            }
            let (done, used) = match available.iter().position(|&b| b == byte) {
                Some(i) => {
                    buf.extend_from_slice(&available[..=i]);
                    (true, i + 1)
                }
                None => {
                    buf.extend_from_slice(available);
                    (false, available.len())
                }
            };
            self.consume(used);
            total += used;
            if done {
                return Ok(total);
            }
        }
    }

    fn read_line(&mut self, buf: &mut String) -> Result<usize> {
        let mut bytes = Vec::new();
        let n = self.read_until(b'\n', &mut bytes)?;
        match core::str::from_utf8(&bytes) {
            Ok(s) => {
                buf.push_str(s);
                Ok(n)
            }
            Err(_) => Err(Error::new(
                ErrorKind::InvalidData,
                "stream did not contain valid UTF-8",
            )),
        }
    }

    fn lines(self) -> Lines<Self>
    where
        Self: Sized,
    {
        Lines { buf: self }
    }
}

// ── BufReader ───────────────────────────────────────────────────────────────

const DEFAULT_BUF_SIZE: usize = 8192;

/// Buffered reader, mirroring std::io::BufReader.
pub struct BufReader<R> {
    inner: R,
    buf: Box<[u8]>,
    pos: usize,
    cap: usize,
}

impl<R: Read> BufReader<R> {
    pub fn new(inner: R) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, inner)
    }

    pub fn with_capacity(capacity: usize, inner: R) -> Self {
        let buf = alloc::vec![0u8; capacity].into_boxed_slice();
        BufReader {
            inner,
            buf,
            pos: 0,
            cap: 0,
        }
    }

    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    pub fn into_inner(self) -> R {
        self.inner
    }

    pub fn buffer(&self) -> &[u8] {
        &self.buf[self.pos..self.cap]
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }
}

impl<R: Read> Read for BufReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // If the request is larger than our buffer and we have no data, bypass
        if self.pos == self.cap && buf.len() >= self.buf.len() {
            return self.inner.read(buf);
        }
        let available = self.fill_buf()?;
        let n = core::cmp::min(available.len(), buf.len());
        buf[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl<R: Read> BufRead for BufReader<R> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        if self.pos >= self.cap {
            self.cap = self.inner.read(&mut self.buf)?;
            self.pos = 0;
        }
        Ok(&self.buf[self.pos..self.cap])
    }

    fn consume(&mut self, amt: usize) {
        self.pos = core::cmp::min(self.pos + amt, self.cap);
    }
}

// ── BufWriter ───────────────────────────────────────────────────────────────

/// Buffered writer, mirroring std::io::BufWriter.
pub struct BufWriter<W: Write> {
    inner: W,
    buf: Vec<u8>,
    capacity: usize,
}

impl<W: Write> BufWriter<W> {
    pub fn new(inner: W) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, inner)
    }

    pub fn with_capacity(capacity: usize, inner: W) -> Self {
        BufWriter {
            inner,
            buf: Vec::with_capacity(capacity),
            capacity,
        }
    }

    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    pub fn buffer(&self) -> &[u8] {
        &self.buf
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn into_inner(self) -> core::result::Result<W, Error> {
        // Skip flushing on into_inner to avoid move issues.
        // Caller should call flush() explicitly before into_inner().
        // Drop impl will flush anyway.
        let mut this = core::mem::ManuallyDrop::new(self);
        let _ = this.flush_buf();
        // Safety: we're consuming self, ManuallyDrop prevents double-drop
        let inner = unsafe { core::ptr::read(&this.inner) };
        Ok(inner)
    }

    fn flush_buf(&mut self) -> Result<()> {
        let mut written = 0;
        let len = self.buf.len();
        while written < len {
            match self.inner.write(&self.buf[written..]) {
                Ok(0) => return Err(Error::from_kind(ErrorKind::WriteZero)),
                Ok(n) => written += n,
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        self.buf.clear();
        Ok(())
    }
}

impl<W: Write> Write for BufWriter<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if self.buf.len() + buf.len() > self.capacity {
            self.flush_buf()?;
        }
        if buf.len() >= self.capacity {
            self.inner.write(buf)
        } else {
            self.buf.extend_from_slice(buf);
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.flush_buf()?;
        self.inner.flush()
    }
}

impl<W: Write> Drop for BufWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush_buf();
    }
}

// ── Cursor ──────────────────────────────────────────────────────────────────

/// In-memory I/O cursor, mirroring std::io::Cursor.
pub struct Cursor<T> {
    inner: T,
    pos: u64,
}

impl<T> Cursor<T> {
    pub fn new(inner: T) -> Self {
        Cursor { inner, pos: 0 }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    pub fn position(&self) -> u64 {
        self.pos
    }

    pub fn set_position(&mut self, pos: u64) {
        self.pos = pos;
    }
}

impl<T: AsRef<[u8]>> Read for Cursor<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let slice = self.inner.as_ref();
        let start = core::cmp::min(self.pos as usize, slice.len());
        let available = &slice[start..];
        let n = core::cmp::min(available.len(), buf.len());
        buf[..n].copy_from_slice(&available[..n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl<T: AsRef<[u8]>> BufRead for Cursor<T> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        let slice = self.inner.as_ref();
        let start = core::cmp::min(self.pos as usize, slice.len());
        Ok(&slice[start..])
    }

    fn consume(&mut self, amt: usize) {
        self.pos += amt as u64;
    }
}

impl<T: AsRef<[u8]>> Seek for Cursor<T> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let (base, offset) = match pos {
            SeekFrom::Start(n) => {
                self.pos = n;
                return Ok(n);
            }
            SeekFrom::End(n) => (self.inner.as_ref().len() as u64, n),
            SeekFrom::Current(n) => (self.pos, n),
        };
        let new_pos = if offset >= 0 {
            base.checked_add(offset as u64)
        } else {
            base.checked_sub((-offset) as u64)
        };
        match new_pos {
            Some(n) => {
                self.pos = n;
                Ok(n)
            }
            None => Err(Error::new(
                ErrorKind::InvalidInput,
                "invalid seek to a negative or overflowing position",
            )),
        }
    }
}

impl Write for Cursor<Vec<u8>> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let pos = self.pos as usize;
        // Extend if needed
        if pos + buf.len() > self.inner.len() {
            self.inner.resize(pos + buf.len(), 0);
        }
        self.inner[pos..pos + buf.len()].copy_from_slice(buf);
        self.pos += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Write for Cursor<&mut Vec<u8>> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let pos = self.pos as usize;
        if pos + buf.len() > self.inner.len() {
            self.inner.resize(pos + buf.len(), 0);
        }
        self.inner[pos..pos + buf.len()].copy_from_slice(buf);
        self.pos += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Write for Cursor<Box<[u8]>> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let pos = self.pos as usize;
        let slice = &mut *self.inner;
        if pos >= slice.len() {
            return Ok(0);
        }
        let n = core::cmp::min(buf.len(), slice.len() - pos);
        slice[pos..pos + n].copy_from_slice(&buf[..n]);
        self.pos += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

// ── Combinator types ────────────────────────────────────────────────────────

/// Iterator over bytes of a reader.
pub struct Bytes<R> {
    inner: R,
}

impl<R: Read> Iterator for Bytes<R> {
    type Item = Result<u8>;

    fn next(&mut self) -> Option<Result<u8>> {
        let mut byte = [0u8; 1];
        loop {
            match self.inner.read(&mut byte) {
                Ok(0) => return None,
                Ok(_) => return Some(Ok(byte[0])),
                Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

/// Chained reader.
pub struct Chain<T, U> {
    first: T,
    second: U,
    done_first: bool,
}

impl<T: Read, U: Read> Read for Chain<T, U> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if !self.done_first {
            match self.first.read(buf)? {
                0 => {
                    self.done_first = true;
                }
                n => return Ok(n),
            }
        }
        self.second.read(buf)
    }
}

/// Limited reader.
pub struct Take<T> {
    inner: T,
    limit: u64,
}

impl<T> Take<T> {
    pub fn limit(&self) -> u64 {
        self.limit
    }

    pub fn set_limit(&mut self, limit: u64) {
        self.limit = limit;
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: Read> Read for Take<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.limit == 0 {
            return Ok(0);
        }
        let max = core::cmp::min(buf.len() as u64, self.limit) as usize;
        let n = self.inner.read(&mut buf[..max])?;
        self.limit -= n as u64;
        Ok(n)
    }
}

/// Line iterator from BufRead.
pub struct Lines<B> {
    buf: B,
}

impl<B: BufRead> Iterator for Lines<B> {
    type Item = Result<String>;

    fn next(&mut self) -> Option<Result<String>> {
        let mut line = String::new();
        match self.buf.read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => {
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                Some(Ok(line))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

// ── Slice/Vec Read/Write impls ──────────────────────────────────────────────

impl Read for &[u8] {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = core::cmp::min(self.len(), buf.len());
        buf[..n].copy_from_slice(&self[..n]);
        *self = &self[n..];
        Ok(n)
    }
}

impl Write for Vec<u8> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Write for &mut [u8] {
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        let n = core::cmp::min(self.len(), data.len());
        let (a, b) = core::mem::take(self).split_at_mut(n);
        a.copy_from_slice(&data[..n]);
        *self = b;
        Ok(n)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

// ── Utility functions ───────────────────────────────────────────────────────

/// Copy all bytes from reader to writer.
pub fn copy<R: Read + ?Sized, W: Write + ?Sized>(reader: &mut R, writer: &mut W) -> Result<u64> {
    let mut buf = [0u8; 8192];
    let mut total = 0u64;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => return Ok(total),
            Ok(n) => n,
            Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        writer.write_all(&buf[..n])?;
        total += n as u64;
    }
}

/// Create a reader that reads nothing (empty).
pub fn empty() -> Empty {
    Empty
}

/// A reader that is always empty.
pub struct Empty;

impl Read for Empty {
    fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }
}

impl BufRead for Empty {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        Ok(&[])
    }
    fn consume(&mut self, _amt: usize) {}
}

/// Create a writer that discards everything.
pub fn sink() -> Sink {
    Sink
}

/// A writer that discards all data.
pub struct Sink;

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Repeat a single byte forever.
pub fn repeat(byte: u8) -> Repeat {
    Repeat(byte)
}

/// A reader that yields the same byte forever.
pub struct Repeat(u8);

impl Read for Repeat {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        for b in buf.iter_mut() {
            *b = self.0;
        }
        Ok(buf.len())
    }
}

// ── Stdin / Stdout / Stderr ─────────────────────────────────────────────────

/// Standard input handle.
pub struct Stdin;

impl Stdin {
    pub fn lock(&self) -> StdinLock<'_> {
        StdinLock {
            _marker: core::marker::PhantomData,
        }
    }

    pub fn read_line(&self, buf: &mut String) -> Result<usize> {
        // Read byte-by-byte from fd 0 until newline
        let mut bytes = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = anyos_std::fs::read(0, &mut byte);
            if n == 0 || n == u32::MAX {
                break;
            }
            bytes.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        match core::str::from_utf8(&bytes) {
            Ok(s) => {
                buf.push_str(s);
                Ok(bytes.len())
            }
            Err(_) => Err(Error::new(
                ErrorKind::InvalidData,
                "stdin contained invalid UTF-8",
            )),
        }
    }
}

impl Read for Stdin {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = anyos_std::fs::read(0, buf);
        if n == u32::MAX {
            return Err(Error::from_kind(ErrorKind::Other));
        }
        Ok(n as usize)
    }
}

pub struct StdinLock<'a> {
    _marker: core::marker::PhantomData<&'a ()>,
}

impl Read for StdinLock<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = anyos_std::fs::read(0, buf);
        if n == u32::MAX {
            return Err(Error::from_kind(ErrorKind::Other));
        }
        Ok(n as usize)
    }
}

/// Standard output handle.
pub struct Stdout;

impl Stdout {
    pub fn lock(&self) -> StdoutLock<'_> {
        StdoutLock {
            _marker: core::marker::PhantomData,
        }
    }
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        anyos_std::fs::write(1, buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

pub struct StdoutLock<'a> {
    _marker: core::marker::PhantomData<&'a ()>,
}

impl Write for StdoutLock<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        anyos_std::fs::write(1, buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Standard error handle.
pub struct Stderr;

impl Stderr {
    pub fn lock(&self) -> StderrLock<'_> {
        StderrLock {
            _marker: core::marker::PhantomData,
        }
    }
}

impl Write for Stderr {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        anyos_std::fs::write(2, buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

pub struct StderrLock<'a> {
    _marker: core::marker::PhantomData<&'a ()>,
}

impl Write for StderrLock<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        anyos_std::fs::write(2, buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Get a handle to standard input.
pub fn stdin() -> Stdin {
    Stdin
}

/// Get a handle to standard output.
pub fn stdout() -> Stdout {
    Stdout
}

/// Get a handle to standard error.
pub fn stderr() -> Stderr {
    Stderr
}
