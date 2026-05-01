#![no_std]
#![no_main]

use anyos_std::format;
use anyos_std::fs;
use anyos_std::sys;
use anyos_std::String;
use anyos_std::Vec;

anyos_std::entry!(main);

fn zeroed_buf(n: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(n);
    v.resize(n, 0);
    v
}

const HELP: &str = "dd - convert and copy data\n\n\
Usage: dd [OPERAND...]\n\n\
Operands:\n\
  if=FILE        read from FILE (default: stdin)\n\
  of=FILE        write to FILE (default: stdout)\n\
  bs=N           set ibs and obs to N bytes (default: 512)\n\
  ibs=N          read block size\n\
  obs=N          write block size\n\
  count=N        copy only N input blocks\n\
  skip=N         skip N ibs-sized blocks at input start\n\
  seek=N         skip N obs-sized blocks at output start\n\
  conv=LIST      comma-separated: notrunc, sync, fsync\n\
  status=LEVEL   none | noxfer | progress (default: print summary)\n\n\
Size suffix: c=1, w=2, b=512, k/K=1024, M=1024^2, G=1024^3,\n\
             kB=1000, MB=10^6, GB=10^9\n";

#[derive(Default)]
struct Conv {
    notrunc: bool,
    sync_pad: bool,
    fsync: bool,
}

#[derive(PartialEq)]
enum Status {
    Default,
    None,
    Noxfer,
    Progress,
}

struct Opts {
    if_path: String,
    of_path: String,
    ibs: u64,
    obs: u64,
    count: Option<u64>,
    skip: u64,
    seek: u64,
    conv: Conv,
    status: Status,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            if_path: String::new(),
            of_path: String::new(),
            ibs: 512,
            obs: 512,
            count: None,
            skip: 0,
            seek: 0,
            conv: Conv::default(),
            status: Status::Default,
        }
    }
}

/// Parse a size with optional SI/IEC suffix.
fn parse_size(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    let mut end = 0;
    while end < b.len() && b[end].is_ascii_digit() {
        end += 1;
    }
    if end == 0 {
        return None;
    }
    let n: u64 = core::str::from_utf8(&b[..end]).ok()?.parse().ok()?;
    let mult: u64 = match &b[end..] {
        b"" => 1,
        b"c" => 1,
        b"w" => 2,
        b"b" => 512,
        b"k" | b"K" | b"KiB" => 1024,
        b"kB" | b"KB" => 1000,
        b"M" | b"MiB" => 1024 * 1024,
        b"MB" => 1_000_000,
        b"G" | b"GiB" => 1024 * 1024 * 1024,
        b"GB" => 1_000_000_000,
        _ => return None,
    };
    n.checked_mul(mult)
}

fn parse_count(s: &str) -> Option<u64> {
    s.parse().ok()
}

fn parse_args(raw: &str) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut bs_set = false;

    for tok in raw.split_whitespace() {
        let (k, v) = match tok.find('=') {
            Some(i) => (&tok[..i], &tok[i + 1..]),
            None => {
                return Err(format!("dd: unknown operand '{}'", tok));
            }
        };
        match k {
            "if" => o.if_path = String::from(v),
            "of" => o.of_path = String::from(v),
            "bs" => {
                let n = parse_size(v).ok_or_else(|| format!("dd: invalid bs='{}'", v))?;
                if n == 0 {
                    return Err(String::from("dd: bs must be > 0"));
                }
                o.ibs = n;
                o.obs = n;
                bs_set = true;
            }
            "ibs" => {
                let n = parse_size(v).ok_or_else(|| format!("dd: invalid ibs='{}'", v))?;
                if n == 0 {
                    return Err(String::from("dd: ibs must be > 0"));
                }
                if !bs_set {
                    o.ibs = n;
                }
            }
            "obs" => {
                let n = parse_size(v).ok_or_else(|| format!("dd: invalid obs='{}'", v))?;
                if n == 0 {
                    return Err(String::from("dd: obs must be > 0"));
                }
                if !bs_set {
                    o.obs = n;
                }
            }
            "count" => {
                o.count = Some(parse_count(v).ok_or_else(|| format!("dd: invalid count='{}'", v))?);
            }
            "skip" => {
                o.skip = parse_count(v).ok_or_else(|| format!("dd: invalid skip='{}'", v))?;
            }
            "seek" => {
                o.seek = parse_count(v).ok_or_else(|| format!("dd: invalid seek='{}'", v))?;
            }
            "conv" => {
                for item in v.split(',') {
                    match item {
                        "notrunc" => o.conv.notrunc = true,
                        "sync" => o.conv.sync_pad = true,
                        "fsync" => o.conv.fsync = true,
                        "" => {}
                        other => {
                            return Err(format!("dd: unsupported conv='{}'", other));
                        }
                    }
                }
            }
            "status" => {
                o.status = match v {
                    "none" => Status::None,
                    "noxfer" => Status::Noxfer,
                    "progress" => Status::Progress,
                    other => {
                        return Err(format!("dd: unsupported status='{}'", other));
                    }
                };
            }
            _ => return Err(format!("dd: unknown operand '{}'", tok)),
        }
    }
    Ok(o)
}

/// Advance `fd` by `bytes` from the current position via repeated lseek calls
/// (lseek takes an i32 offset, so large jumps need multiple steps).
fn seek_forward(fd: u32, bytes: u64) -> bool {
    let mut remaining = bytes;
    while remaining > 0 {
        let step = remaining.min(i32::MAX as u64) as i32;
        let r = fs::lseek(fd, step, fs::SEEK_CUR);
        if r == u32::MAX {
            return false;
        }
        remaining -= step as u64;
    }
    true
}

/// For input files we cannot seek, read-and-discard up to `bytes`.
fn skip_by_reading(fd: u32, bytes: u64, ibs: u64) -> u64 {
    let mut buf = zeroed_buf(ibs as usize);
    let mut left = bytes;
    let mut skipped = 0u64;
    while left > 0 {
        let want = left.min(ibs) as usize;
        let n = fs::read(fd, &mut buf[..want]);
        if n == 0 || n == u32::MAX {
            break;
        }
        let n = n as u64;
        skipped += n;
        left = left.saturating_sub(n);
    }
    skipped
}

/// Write the entire buffer, splitting large buffers into syscall-sized chunks.
/// Returns true on full write.
fn write_all(fd: u32, buf: &[u8]) -> bool {
    // Keep syscall chunks bounded so huge obs values don't hit kernel limits.
    const CHUNK: usize = 64 * 1024;
    let mut off = 0;
    while off < buf.len() {
        let end = (off + CHUNK).min(buf.len());
        let n = fs::write(fd, &buf[off..end]);
        if n == u32::MAX || n == 0 {
            return false;
        }
        off += n as usize;
    }
    true
}

fn format_bytes(b: u64, out: &mut String) {
    use core::fmt::Write;
    if b >= 1024 * 1024 * 1024 {
        let _ = write!(
            out,
            "{}.{} GiB",
            b / (1 << 30),
            (b % (1 << 30)) / ((1 << 30) / 10)
        );
    } else if b >= 1024 * 1024 {
        let _ = write!(
            out,
            "{}.{} MiB",
            b / (1 << 20),
            (b % (1 << 20)) / ((1 << 20) / 10)
        );
    } else if b >= 1024 {
        let _ = write!(out, "{}.{} KiB", b / 1024, (b % 1024) / 102);
    } else {
        let _ = write!(out, "{} B", b);
    }
}

fn print_summary(
    records_in_full: u64,
    records_in_part: u64,
    records_out_full: u64,
    records_out_part: u64,
    bytes: u64,
    ms: u32,
    status: &Status,
) {
    if *status == Status::None {
        return;
    }
    anyos_std::println!("{}+{} records in", records_in_full, records_in_part);
    anyos_std::println!("{}+{} records out", records_out_full, records_out_part);

    if *status == Status::Noxfer {
        return;
    }

    let mut human = String::new();
    format_bytes(bytes, &mut human);
    let secs_whole = ms / 1000;
    let secs_frac = (ms % 1000) / 10; // hundredths
    if ms == 0 {
        anyos_std::println!("{} bytes ({}) copied, 0.00 s", bytes, human);
    } else {
        // bytes per second (avoid div-by-zero, integer math)
        let bps = bytes.saturating_mul(1000) / ms as u64;
        let mut rate = String::new();
        format_bytes(bps, &mut rate);
        anyos_std::println!(
            "{} bytes ({}) copied, {}.{:02} s, {}/s",
            bytes,
            human,
            secs_whole,
            secs_frac,
            rate
        );
    }
}

fn run(o: &Opts) -> i32 {
    // Open input
    let in_fd = if o.if_path.is_empty() {
        0
    } else {
        let fd = fs::open(&o.if_path, 0);
        if fd == u32::MAX {
            anyos_std::println!("dd: cannot open '{}' for reading", o.if_path);
            return 1;
        }
        fd
    };

    // Open output
    let out_fd = if o.of_path.is_empty() {
        1
    } else {
        let mut flags = fs::O_WRITE | fs::O_CREATE;
        if !o.conv.notrunc && o.seek == 0 {
            flags |= fs::O_TRUNC;
        }
        let fd = fs::open(&o.of_path, flags);
        if fd == u32::MAX {
            anyos_std::println!("dd: cannot open '{}' for writing", o.of_path);
            if in_fd != 0 {
                fs::close(in_fd);
            }
            return 1;
        }
        fd
    };

    // skip / seek
    if o.skip > 0 {
        let bytes = o.skip.saturating_mul(o.ibs);
        let ok = if in_fd == 0 {
            skip_by_reading(in_fd, bytes, o.ibs) == bytes
        } else if !seek_forward(in_fd, bytes) {
            // Fallback: some block devices reject lseek — read-and-discard.
            skip_by_reading(in_fd, bytes, o.ibs) == bytes
        } else {
            true
        };
        if !ok {
            anyos_std::println!("dd: skip beyond end of input");
        }
    }
    if o.seek > 0 {
        let bytes = o.seek.saturating_mul(o.obs);
        if !seek_forward(out_fd, bytes) {
            anyos_std::println!("dd: seek failed on output");
            if in_fd != 0 {
                fs::close(in_fd);
            }
            if out_fd != 1 {
                fs::close(out_fd);
            }
            return 1;
        }
    }

    let mut buf = zeroed_buf(o.ibs as usize);
    let mut records_in_full: u64 = 0;
    let mut records_in_part: u64 = 0;
    let mut records_out_full: u64 = 0;
    let mut records_out_part: u64 = 0;
    let mut bytes_total: u64 = 0;

    let start_ms = sys::uptime_ms();
    let mut last_progress = start_ms;

    loop {
        if let Some(c) = o.count {
            if records_in_full + records_in_part >= c {
                break;
            }
        }

        let n = fs::read(in_fd, &mut buf[..]);
        if n == u32::MAX {
            anyos_std::println!("dd: read error");
            break;
        }
        if n == 0 {
            break;
        }
        let n = n as usize;

        let out_len = if n as u64 == o.ibs {
            records_in_full += 1;
            n
        } else {
            records_in_part += 1;
            if o.conv.sync_pad {
                // Zero-pad short reads to full ibs.
                for b in &mut buf[n..] {
                    *b = 0;
                }
                o.ibs as usize
            } else {
                n
            }
        };

        if !write_all(out_fd, &buf[..out_len]) {
            anyos_std::println!("dd: write error");
            break;
        }

        bytes_total += out_len as u64;
        if out_len as u64 == o.obs {
            records_out_full += 1;
        } else {
            records_out_part += 1;
        }

        if o.status == Status::Progress {
            let now = sys::uptime_ms();
            if now.wrapping_sub(last_progress) >= 1000 {
                last_progress = now;
                let mut human = String::new();
                format_bytes(bytes_total, &mut human);
                anyos_std::println!("{} bytes ({}) copied", bytes_total, human);
            }
        }
    }

    let end_ms = sys::uptime_ms();

    if o.conv.fsync {
        fs::fsync(out_fd as i32);
    }
    if in_fd != 0 {
        fs::close(in_fd);
    }
    if out_fd != 1 {
        fs::close(out_fd);
    }

    print_summary(
        records_in_full,
        records_in_part,
        records_out_full,
        records_out_part,
        bytes_total,
        end_ms.wrapping_sub(start_ms),
        &o.status,
    );
    0
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let raw = raw.trim();

    if raw == "--help" || raw == "-h" {
        anyos_std::print!("{}", HELP);
        return;
    }

    match parse_args(raw) {
        Ok(o) => {
            run(&o);
        }
        Err(e) => {
            anyos_std::println!("{}", e);
            anyos_std::println!("Try 'dd --help' for usage.");
        }
    }
}
