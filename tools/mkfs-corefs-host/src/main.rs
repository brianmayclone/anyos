// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `mkfs-corefs-host` — host-side CoreFS formatter.
//!
//! Creates a CoreFS volume inside an existing file, loopback device or
//! `/dev/sdX`.  Designed for the anyOS image-build pipeline (`mkimage`
//! calls this tool to format the second partition as CoreFS) but usable
//! standalone for offline disk preparation and testing.
//!
//! ```text
//! mkfs-corefs-host --output <path>
//!                  [--offset <bytes>]   (default: 0)
//!                  [--size   <bytes>]   (default: file length - offset)
//!                  [--label  <name>]    (default: "corefs")
//!                  [--inodes <count>]
//!                  [--journal-blocks <n>]
//!                  [--json]
//!                  [--help]
//! ```
//!
//! All numeric options accept the `k`/`K`/`m`/`M`/`g`/`G`/`t`/`T`
//! suffixes (powers of 1024) via the shared parser in
//! `libcorefs_tools::args`.

mod backend;
mod format;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use libcorefs_tools::args::{self, LongArgs};
use libcorefs_tools::error::ExitCode as ToolExit;

use crate::format::{format_volume, FormatError, FormatOutcome, FormatRequest};

fn main() -> ExitCode {
    // Collect `std::env::args` into a single whitespace-joined string so
    // we can reuse the argument parser from libcorefs-tools (designed
    // for the anyOS `process::args` flat buffer).
    let joined = std::env::args()
        .skip(1)
        .map(|a| shell_quote(&a))
        .collect::<Vec<_>>()
        .join(" ");
    let parsed = args::parse(&joined);

    if parsed.has("help") || parsed.has("h") {
        print_usage();
        return ExitCode::from(ToolExit::Success.as_u32() as u8);
    }

    let cli = match Cli::from_args(&parsed) {
        Ok(cli) => cli,
        Err(msg) => {
            eprintln!("mkfs-corefs-host: {msg}");
            eprintln!();
            print_usage();
            return ExitCode::from(ToolExit::InvalidArgument.as_u32() as u8);
        }
    };

    match run(&cli) {
        Ok(outcome) => {
            emit_report(&cli, &outcome);
            ExitCode::from(ToolExit::Success.as_u32() as u8)
        }
        Err(e) => {
            eprintln!("mkfs-corefs-host: {e}");
            let code = match e {
                FormatError::Io { .. } => ToolExit::IoError,
                FormatError::InvalidSize { .. }
                | FormatError::InvalidOffset { .. }
                | FormatError::SizeOverflow => ToolExit::InvalidArgument,
                FormatError::CoreFs(ref inner) => libcorefs_tools::error::exit_code_for(inner),
            };
            ExitCode::from(code.as_u32() as u8)
        }
    }
}

/// Parsed + validated command line.
#[derive(Debug, Clone)]
struct Cli {
    output: PathBuf,
    offset: u64,
    size: Option<u64>,
    label: String,
    inode_count: Option<u64>,
    journal_blocks: Option<u64>,
    json: bool,
}

impl Cli {
    fn from_args(a: &LongArgs) -> Result<Self, String> {
        let output = a
            .get("output")
            .or_else(|| a.get("o"))
            .ok_or_else(|| String::from("missing --output <path>"))?;
        if output.is_empty() {
            return Err(String::from("--output must not be empty"));
        }

        let offset = a.get_u64("offset").unwrap_or(0);
        let size = a.get_u64("size");
        let label = a.get("label").unwrap_or("corefs").to_string();
        if label.as_bytes().len() > 16 {
            return Err(format!(
                "--label '{label}' exceeds the 16-byte volume label limit"
            ));
        }
        let inode_count = a.get_u64("inodes");
        let journal_blocks = a.get_u64("journal-blocks");
        let json = a.has("json");

        Ok(Self {
            output: PathBuf::from(output),
            offset,
            size,
            label,
            inode_count,
            journal_blocks,
            json,
        })
    }
}

/// Resolve `--size` (falling back to file-length minus offset) and
/// dispatch into [`format_volume`].
fn run(cli: &Cli) -> Result<FormatOutcome, FormatError> {
    let size = resolve_size(&cli.output, cli.offset, cli.size)?;
    let req = FormatRequest {
        output: &cli.output,
        offset_bytes: cli.offset,
        size_bytes: size,
        label: &cli.label,
        inode_count: cli.inode_count,
        journal_blocks: cli.journal_blocks,
    };
    format_volume(&req)
}

/// Determine the partition size from explicit `--size` or the
/// current length of the output file.
fn resolve_size(
    output: &Path,
    offset: u64,
    explicit: Option<u64>,
) -> Result<u64, FormatError> {
    if let Some(s) = explicit {
        return Ok(s);
    }
    let len = output
        .metadata()
        .map(|m| m.len())
        .map_err(|e| FormatError::Io {
            path: output.display().to_string(),
            source: e,
        })?;
    if len <= offset {
        return Err(FormatError::InvalidSize {
            size: 0,
            sector_size: 512,
        });
    }
    Ok(len - offset)
}

/// Text or JSON summary.
fn emit_report(cli: &Cli, outcome: &FormatOutcome) {
    if cli.json {
        println!(
            "{{\"output\":\"{path}\",\"offset_bytes\":{off},\"capacity_bytes\":{cap},\
             \"total_blocks\":{blocks},\"inode_slots\":{inodes},\
             \"journal_blocks\":{journal},\"label\":\"{label}\",\"generation\":{gen}}}",
            path = json_escape(&cli.output.display().to_string()),
            off = cli.offset,
            cap = outcome.capacity_bytes,
            blocks = outcome.total_blocks,
            inodes = outcome.inode_count,
            journal = outcome.journal_blocks,
            label = json_escape(&outcome.label),
            gen = outcome.generation,
        );
    } else {
        println!(
            "mkfs-corefs-host: formatted {path}",
            path = cli.output.display()
        );
        println!("  offset          : {} bytes", cli.offset);
        println!("  capacity        : {} bytes", outcome.capacity_bytes);
        println!("  total blocks    : {}", outcome.total_blocks);
        println!("  inode slots     : {}", outcome.inode_count);
        println!("  journal blocks  : {}", outcome.journal_blocks);
        println!("  label           : {}", outcome.label);
        println!("  generation      : {}", outcome.generation);
    }
}

fn print_usage() {
    eprintln!(
        "Usage: mkfs-corefs-host --output <path>\n\
         \n\
         Options:\n\
           --output, -o <path>       Disk image or device to format\n\
           --offset <bytes>          Byte offset inside <path> (default: 0)\n\
           --size <bytes>            Partition size (default: file length - offset)\n\
           --label <name>            Volume label, up to 16 bytes (default: corefs)\n\
           --inodes <count>          Override inode-table size\n\
           --journal-blocks <n>      Override journal size in blocks\n\
           --json                    Emit JSON instead of plain text\n\
           --help, -h                Show this help\n\
         \n\
         Numeric values accept k/K, m/M, g/G, t/T suffixes (powers of 1024)."
    );
}

/// Minimal JSON string-escape (backslashes + quotes + control chars).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Lightweight shell-like quoting so we can feed the joined argv back
/// through the `libcorefs_tools::args` parser without losing tokens
/// that contain whitespace.  Only `double-quoting` is applied — the
/// parser's tokenizer already handles embedded quotes.
fn shell_quote(arg: &str) -> String {
    if arg.contains(char::is_whitespace) {
        format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        arg.to_string()
    }
}
