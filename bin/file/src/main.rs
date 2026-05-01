//! file — determine file type, like Linux `file(1)`.
//!
//! Usage: file <path> [path ...]
//!
//! Detection order (same as GNU file):
//!   1. Special filesystem types (directory, symlink, device)
//!   2. Magic bytes — binary format identification
//!   3. Text analysis — encoding and content heuristics

#![no_std]
#![no_main]

anyos_std::entry!(main);

// ─── Magic signatures ─────────────────────────────────────────────────────────

struct Magic {
    offset: usize,
    bytes: &'static [u8],
    desc: &'static str,
}

// Ordered from most-specific to least-specific.
static MAGIC: &[Magic] = &[
    // ELF
    Magic {
        offset: 0,
        bytes: b"\x7FELF",
        desc: "ELF executable",
    },
    // Scripts / text interpreters
    Magic {
        offset: 0,
        bytes: b"#!/bin/sh",
        desc: "POSIX shell script",
    },
    Magic {
        offset: 0,
        bytes: b"#!/bin/bash",
        desc: "Bash shell script",
    },
    Magic {
        offset: 0,
        bytes: b"#!/usr/bin/env",
        desc: "script",
    },
    Magic {
        offset: 0,
        bytes: b"#!",
        desc: "script",
    },
    // Archives / compressed
    Magic {
        offset: 0,
        bytes: b"PK\x03\x04",
        desc: "Zip archive",
    },
    Magic {
        offset: 0,
        bytes: b"PK\x05\x06",
        desc: "Zip archive (empty)",
    },
    Magic {
        offset: 0,
        bytes: b"\x1F\x8B",
        desc: "gzip compressed data",
    },
    Magic {
        offset: 0,
        bytes: b"BZh",
        desc: "bzip2 compressed data",
    },
    Magic {
        offset: 0,
        bytes: b"\xFD7zXZ\x00",
        desc: "XZ compressed data",
    },
    Magic {
        offset: 0,
        bytes: b"7z\xBC\xAF\x27\x1C",
        desc: "7-zip archive",
    },
    Magic {
        offset: 0,
        bytes: b"Rar!\x1A\x07",
        desc: "RAR archive",
    },
    Magic {
        offset: 0,
        bytes: b"ustar",
        desc: "POSIX tar archive",
    },
    Magic {
        offset: 257,
        bytes: b"ustar",
        desc: "POSIX tar archive",
    },
    // Images
    Magic {
        offset: 0,
        bytes: b"\x89PNG\r\n\x1A\n",
        desc: "PNG image",
    },
    Magic {
        offset: 0,
        bytes: b"\xFF\xD8\xFF",
        desc: "JPEG image",
    },
    Magic {
        offset: 0,
        bytes: b"GIF87a",
        desc: "GIF image (87a)",
    },
    Magic {
        offset: 0,
        bytes: b"GIF89a",
        desc: "GIF image (89a)",
    },
    Magic {
        offset: 0,
        bytes: b"BM",
        desc: "BMP image",
    },
    Magic {
        offset: 0,
        bytes: b"RIFF",
        desc: "RIFF data",
    }, // WAV/AVI
    Magic {
        offset: 0,
        bytes: b"\x00\x00\x01\x00",
        desc: "Windows ICO image",
    },
    Magic {
        offset: 0,
        bytes: b"II\x2A\x00",
        desc: "TIFF image (little-endian)",
    },
    Magic {
        offset: 0,
        bytes: b"MM\x00\x2A",
        desc: "TIFF image (big-endian)",
    },
    Magic {
        offset: 0,
        bytes: b"\x00\x00\x00\x0CJXL\x20\x0D\x0A\x87\x0A",
        desc: "JPEG XL image",
    },
    Magic {
        offset: 0,
        bytes: b"\xFF\x0A",
        desc: "JPEG XL image (codestream)",
    },
    // Documents
    Magic {
        offset: 0,
        bytes: b"%PDF-",
        desc: "PDF document",
    },
    Magic {
        offset: 0,
        bytes: b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1",
        desc: "Microsoft Office document (OLE)",
    },
    // Fonts
    Magic {
        offset: 0,
        bytes: b"\x00\x01\x00\x00\x00",
        desc: "TrueType font",
    },
    Magic {
        offset: 0,
        bytes: b"OTTO",
        desc: "OpenType font",
    },
    Magic {
        offset: 0,
        bytes: b"ttcf",
        desc: "TrueType font collection",
    },
    Magic {
        offset: 0,
        bytes: b"wOFF",
        desc: "WOFF font",
    },
    Magic {
        offset: 0,
        bytes: b"wOF2",
        desc: "WOFF2 font",
    },
    // Executables / object files
    Magic {
        offset: 0,
        bytes: b"MZ",
        desc: "MS-DOS / PE executable",
    },
    Magic {
        offset: 0,
        bytes: b"\xCE\xFA\xED\xFE",
        desc: "Mach-O binary (32-bit)",
    },
    Magic {
        offset: 0,
        bytes: b"\xCF\xFA\xED\xFE",
        desc: "Mach-O binary (64-bit)",
    },
    Magic {
        offset: 0,
        bytes: b"\xFE\xED\xFA\xCE",
        desc: "Mach-O binary (32-bit, BE)",
    },
    Magic {
        offset: 0,
        bytes: b"\xFE\xED\xFA\xCF",
        desc: "Mach-O binary (64-bit, BE)",
    },
    // Audio / video
    Magic {
        offset: 0,
        bytes: b"fLaC",
        desc: "FLAC audio",
    },
    Magic {
        offset: 0,
        bytes: b"OggS",
        desc: "Ogg data",
    },
    Magic {
        offset: 0,
        bytes: b"ID3",
        desc: "MP3 audio (ID3 tagged)",
    },
    Magic {
        offset: 0,
        bytes: b"\xFF\xFB",
        desc: "MP3 audio",
    },
    Magic {
        offset: 0,
        bytes: b"\xFF\xF3",
        desc: "MP3 audio",
    },
    Magic {
        offset: 0,
        bytes: b"\xFF\xF2",
        desc: "MP3 audio",
    },
    // Databases / misc
    Magic {
        offset: 0,
        bytes: b"SQLite format 3\x00",
        desc: "SQLite database",
    },
    Magic {
        offset: 0,
        bytes: b"\x7FANY",
        desc: "anyOS binary",
    },
];

// ─── Read helper ─────────────────────────────────────────────────────────────

const READ_SIZE: usize = 512;

fn read_header(path: &str, buf: &mut [u8; READ_SIZE]) -> usize {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return 0;
    }
    let n = anyos_std::fs::read(fd, buf);
    anyos_std::fs::close(fd);
    if n == u32::MAX {
        0
    } else {
        n as usize
    }
}

// ─── Magic matching ──────────────────────────────────────────────────────────

fn match_magic(data: &[u8]) -> Option<&'static str> {
    for m in MAGIC {
        let end = m.offset + m.bytes.len();
        if end <= data.len() && &data[m.offset..end] == m.bytes {
            return Some(m.desc);
        }
    }
    None
}

// ─── Text analysis ───────────────────────────────────────────────────────────

/// Returns true if the byte looks like it could appear in UTF-8 text.
fn is_text_byte(b: u8) -> bool {
    b >= 0x20 || b == b'\n' || b == b'\r' || b == b'\t' || b == 0x0C || b == 0x1B
}

#[derive(PartialEq)]
enum TextKind {
    Ascii,
    Utf8,
    Binary,
}

fn classify_text(data: &[u8]) -> TextKind {
    if data.is_empty() {
        return TextKind::Ascii;
    }

    let mut i = 0;
    let mut non_ascii = false;
    while i < data.len() {
        let b = data[i];
        // Check for null bytes or non-printable control chars → binary
        if b == 0x00 || (b < 0x20 && !is_text_byte(b)) {
            return TextKind::Binary;
        }
        if b < 0x80 {
            i += 1;
            continue;
        }
        non_ascii = true;
        // Try to decode a UTF-8 multi-byte sequence
        let seq_len = if b & 0xE0 == 0xC0 {
            2
        } else if b & 0xF0 == 0xE0 {
            3
        } else if b & 0xF8 == 0xF0 {
            4
        } else {
            return TextKind::Binary;
        };
        if i + seq_len > data.len() {
            break;
        }
        for k in 1..seq_len {
            if data[i + k] & 0xC0 != 0x80 {
                return TextKind::Binary;
            }
        }
        i += seq_len;
    }
    if non_ascii {
        TextKind::Utf8
    } else {
        TextKind::Ascii
    }
}

// ─── Text content heuristics ─────────────────────────────────────────────────

fn sniff_text(data: &[u8]) -> &'static str {
    // Look for common text file signatures in the first line
    if starts_with_tag(data, b"<?xml") {
        return "XML document";
    }
    if starts_with_tag(data, b"<html") {
        return "HTML document";
    }
    if starts_with_tag(data, b"<HTML") {
        return "HTML document";
    }
    if starts_with_tag(data, b"<!DOCTYPE html") {
        return "HTML document";
    }
    if starts_with_tag(data, b"<!DOCTYPE HTML") {
        return "HTML document";
    }
    if starts_with_tag(data, b"{") {
        return "JSON data";
    }
    if starts_with_tag(data, b"[") {
        return "JSON data";
    }
    if starts_with_tag(data, b"---") {
        return "YAML data";
    }
    if starts_with_tag(data, b"[package]") || starts_with_tag(data, b"[dependencies]") {
        return "TOML data";
    }
    if contains_early(data, b"fn main()") || contains_early(data, b"fn main() {") {
        return "Rust source";
    }
    if contains_early(data, b"int main(") {
        return "C source";
    }
    if contains_early(data, b"#include ") {
        return "C source";
    }
    if contains_early(data, b"package main") || contains_early(data, b"func main()") {
        return "Go source";
    }
    if contains_early(data, b"import java") || contains_early(data, b"public class") {
        return "Java source";
    }
    if contains_early(data, b"def ") && contains_early(data, b"import ") {
        return "Python source";
    }
    if contains_early(data, b"def ") {
        return "Python source";
    }
    if starts_with_tag(data, b"[") && contains_early(data, b"=") {
        return "INI configuration";
    }
    "text"
}

fn starts_with_tag(data: &[u8], tag: &[u8]) -> bool {
    // Skip leading whitespace / BOM
    let mut start = 0;
    while start < data.len()
        && (data[start] == b' '
            || data[start] == b'\t'
            || data[start] == b'\r'
            || data[start] == b'\n')
    {
        start += 1;
    }
    let data = &data[start..];
    data.len() >= tag.len() && data[..tag.len()].eq_ignore_ascii_case(tag)
}

fn contains_early(data: &[u8], needle: &[u8]) -> bool {
    let limit = data.len().min(2048);
    let haystack = &data[..limit];
    if needle.len() > haystack.len() {
        return false;
    }
    for i in 0..=(haystack.len() - needle.len()) {
        if &haystack[i..i + needle.len()] == needle {
            return true;
        }
    }
    false
}

// ─── ELF details ─────────────────────────────────────────────────────────────

fn describe_elf(data: &[u8]) -> &'static str {
    if data.len() < 18 {
        return "ELF executable";
    }
    let class = data[4]; // 1=32-bit, 2=64-bit
    let etype = u16::from_le_bytes([data[16], data[17]]);
    match (class, etype) {
        (1, 1) => "ELF 32-bit relocatable",
        (1, 2) => "ELF 32-bit executable",
        (1, 3) => "ELF 32-bit shared library",
        (1, 4) => "ELF 32-bit core dump",
        (2, 1) => "ELF 64-bit relocatable",
        (2, 2) => "ELF 64-bit executable",
        (2, 3) => "ELF 64-bit shared library",
        (2, 4) => "ELF 64-bit core dump",
        _ => "ELF executable",
    }
}

// ─── Main detection logic ─────────────────────────────────────────────────────

fn describe_file(path: &str) -> &'static str {
    // Stat the path first
    let mut stat_buf = [0u32; 7];
    if anyos_std::fs::stat(path, &mut stat_buf) == u32::MAX {
        return "ERROR: cannot stat";
    }
    let ftype = stat_buf[0];
    let is_sym = stat_buf[2] & 1 != 0;

    if is_sym {
        return "symbolic link";
    }
    if ftype == 1 {
        return "directory";
    }
    if ftype == 2 {
        return "device";
    }

    // Read header bytes
    static mut HBUF: [u8; READ_SIZE] = [0u8; READ_SIZE];
    let n = unsafe {
        let buf = &mut HBUF;
        read_header(path, buf)
    };
    if n == 0 {
        return "empty";
    }
    let data = unsafe { &HBUF[..n] };

    // ELF: give detailed description
    if data.len() >= 4 && &data[..4] == b"\x7FELF" {
        return describe_elf(data);
    }

    // Magic byte matching
    if let Some(desc) = match_magic(data) {
        return desc;
    }

    // Text / binary classification
    match classify_text(data) {
        TextKind::Binary => "data",
        TextKind::Ascii => {
            let t = sniff_text(data);
            if t == "text" {
                "ASCII text"
            } else {
                t
            }
        }
        TextKind::Utf8 => {
            let t = sniff_text(data);
            if t == "text" {
                "UTF-8 Unicode text"
            } else {
                t
            }
        }
    }
}

// ─── Output helpers ───────────────────────────────────────────────────────────

fn print_result(path: &str, desc: &str) {
    anyos_std::print!("{}: {}\n", path, desc);
}

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if raw.contains("--help") {
        anyos_std::println!("file - Identify file type\n\nUsage: file FILE...");
        return;
    }

    if args.pos_count == 0 {
        anyos_std::print!("Usage: file <file> [file ...]\n");
        return;
    }

    for i in 0..args.pos_count {
        let path = args.positional[i];
        if path.is_empty() {
            continue;
        }
        let desc = describe_file(path);
        print_result(path, desc);
    }
}
