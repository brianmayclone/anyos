// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `fusedemo` — minimal hello-world FUSE daemon.
//!
//! Reference implementation that demonstrates the FUSE integration is
//! protocol-agnostic: it speaks the `corefs-fuse-proto` wire format but
//! carries no CoreFS dependency. The exported filesystem is a read-only
//! in-memory layout:
//!
//! - `/`         — root directory (ino 1)
//! - `/hello.txt` — static "Hello, world!\n" file (ino 2)
//!
//! Useful for validating `/dev/fuse` IPC, the session loop in
//! `corefs-fuse-adapter` and VFS integration without pulling in the
//! CoreFS stack.

#![no_std]
#![no_main]

use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use corefs_fuse_adapter::{FuseHandler, HandlerResult, SessionLoop, Transport};
use corefs_fuse_proto::{
    Attr, DirEntry, FrameHeader, PROTOCOL_VERSION, Reply, ReplyFrame, Request, RequestFrame,
    StatFs,
};

anyos_std::entry!(main);

const ENOENT: i32 = 2;
const EISDIR: i32 = 21;
const ENOSYS: i32 = 38;
const DEV_FUSE: &str = "/dev/fuse";
const MAX_FRAME: usize = 16 * 1024;

const ROOT_INO: u64 = 1;
const HELLO_INO: u64 = 2;
const HELLO_NAME: &str = "hello.txt";
const HELLO_BODY: &[u8] = b"Hello, world!\n";

// ---------------------------------------------------------------------------
// HelloHandler — stateless, read-only hello-world FS
// ---------------------------------------------------------------------------

struct HelloHandler;

fn root_attr() -> Attr {
    Attr {
        ino: ROOT_INO,
        size: 0,
        blocks: 0,
        mode: 0o40755,
        nlink: 2,
        uid: 0,
        gid: 0,
        kind: 2, // Directory
        crtime_secs: 0,
        mtime_secs: 0,
        ctime_secs: 0,
    }
}

fn hello_attr() -> Attr {
    Attr {
        ino: HELLO_INO,
        size: HELLO_BODY.len() as u64,
        blocks: 1,
        mode: 0o100444,
        nlink: 1,
        uid: 0,
        gid: 0,
        kind: 1, // File
        crtime_secs: 0,
        mtime_secs: 0,
        ctime_secs: 0,
    }
}

impl FuseHandler for HelloHandler {
    fn handle(&mut self, request: Request) -> HandlerResult {
        match request {
            Request::Init { .. } => HandlerResult::Ok(Reply::Init {
                version: PROTOCOL_VERSION,
            }),
            Request::Destroy => HandlerResult::Ok(Reply::Destroy),
            Request::Getattr { ino } => match ino {
                ROOT_INO => HandlerResult::Ok(Reply::Getattr(root_attr())),
                HELLO_INO => HandlerResult::Ok(Reply::Getattr(hello_attr())),
                _ => err(ENOENT, "no such inode"),
            },
            Request::Lookup { parent, name } => {
                if parent != ROOT_INO {
                    return err(ENOENT, "unknown parent");
                }
                if name == HELLO_NAME {
                    HandlerResult::Ok(Reply::Lookup(hello_attr()))
                } else {
                    err(ENOENT, "no such entry")
                }
            }
            Request::Readdir { ino, offset } => {
                if ino != ROOT_INO {
                    return err(ENOENT, "readdir on non-directory");
                }
                if offset > 0 {
                    return HandlerResult::Ok(Reply::Readdir(Vec::new()));
                }
                HandlerResult::Ok(Reply::Readdir(vec![DirEntry {
                    ino: HELLO_INO,
                    kind: 1,
                    name: HELLO_NAME.to_string(),
                    next_offset: 1,
                }]))
            }
            Request::Open { ino, .. } => match ino {
                HELLO_INO => HandlerResult::Ok(Reply::Open {
                    fh: 1,
                    flags: 0,
                }),
                ROOT_INO => err(EISDIR, "cannot open directory as file"),
                _ => err(ENOENT, "no such inode"),
            },
            Request::Read {
                ino,
                offset,
                size,
                ..
            } => {
                if ino != HELLO_INO {
                    return err(ENOENT, "read on non-file");
                }
                let start = offset as usize;
                if start >= HELLO_BODY.len() {
                    return HandlerResult::Ok(Reply::Read { data: Vec::new() });
                }
                let end = (start + size as usize).min(HELLO_BODY.len());
                HandlerResult::Ok(Reply::Read {
                    data: HELLO_BODY[start..end].to_vec(),
                })
            }
            Request::Release { .. } => HandlerResult::Ok(Reply::Release),
            Request::Statfs => HandlerResult::Ok(Reply::Statfs(StatFs {
                bsize: 4096,
                blocks: 1,
                bfree: 0,
                bavail: 0,
                files: 2,
                ffree: 0,
                namelen: 255,
            })),
            Request::Flush { .. } => HandlerResult::Ok(Reply::Flush),
            Request::Fsync { .. } => HandlerResult::Ok(Reply::Fsync),
            // Read-only FS: reject every mutating op honestly.
            Request::Write { .. }
            | Request::Create { .. }
            | Request::Mkdir { .. }
            | Request::Unlink { .. }
            | Request::Setattr { .. }
            | Request::Rmdir { .. }
            | Request::Rename { .. }
            | Request::Symlink { .. } => err(ENOSYS, "read-only demo fs"),
            Request::Readlink { .. } => err(ENOSYS, "no symlinks in demo fs"),
        }
    }
}

fn err(errno: i32, msg: &str) -> HandlerResult {
    HandlerResult::Err {
        errno,
        message: Some(msg.to_string()),
    }
}

// ---------------------------------------------------------------------------
// DevFuseTransport — thin copy of corefsd's /dev/fuse transport
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum DevFuseError {
    OpenFailed,
    ShortFrame,
    DecodeFailed,
    EncodeFailed,
    FrameTooLarge,
    WriteFailed,
}

struct DevFuseTransport {
    fd: u32,
    scratch: Vec<u8>,
}

impl DevFuseTransport {
    fn open() -> Result<Self, DevFuseError> {
        let fd = anyos_std::fs::open(DEV_FUSE, anyos_std::fs::O_WRITE);
        if fd == u32::MAX {
            return Err(DevFuseError::OpenFailed);
        }
        Ok(Self {
            fd,
            scratch: vec![0u8; MAX_FRAME],
        })
    }
}

impl Drop for DevFuseTransport {
    fn drop(&mut self) {
        if self.fd != u32::MAX {
            let _ = anyos_std::fs::close(self.fd);
        }
    }
}

impl Transport for DevFuseTransport {
    type Error = DevFuseError;

    fn recv_request(&mut self) -> Result<Option<RequestFrame>, Self::Error> {
        let n = anyos_std::fs::read(self.fd, &mut self.scratch);
        if n == u32::MAX {
            return Err(DevFuseError::ShortFrame);
        }
        let n = n as usize;
        if n == 0 {
            return Ok(None);
        }
        if n < 8 {
            return Err(DevFuseError::ShortFrame);
        }
        let mut unique_bytes = [0u8; 8];
        unique_bytes.copy_from_slice(&self.scratch[0..8]);
        let unique = u64::from_le_bytes(unique_bytes);
        let body = &self.scratch[8..n];
        let request: Request = bincode::serde::decode_from_slice::<Request, _>(
            body,
            bincode::config::legacy(),
        )
        .map(|(v, _)| v)
        .map_err(|_| DevFuseError::DecodeFailed)?;
        Ok(Some(RequestFrame {
            header: FrameHeader { unique },
            op: request,
        }))
    }

    fn send_reply(&mut self, reply: &ReplyFrame) -> Result<(), Self::Error> {
        let body = bincode::serde::encode_to_vec(&reply.payload, bincode::config::legacy())
            .map_err(|_| DevFuseError::EncodeFailed)?;
        let total = 8 + body.len();
        if total > MAX_FRAME {
            return Err(DevFuseError::FrameTooLarge);
        }
        let mut frame: Vec<u8> = Vec::with_capacity(total);
        frame.extend_from_slice(&reply.header.unique.to_le_bytes());
        frame.extend_from_slice(&body);
        let n = anyos_std::fs::write(self.fd, &frame);
        if n == u32::MAX {
            return Err(DevFuseError::WriteFailed);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() -> u32 {
    anyos_std::println!("fusedemo: hello-world FUSE daemon");
    anyos_std::println!("fusedemo: protocol version {}", PROTOCOL_VERSION);
    let transport = match DevFuseTransport::open() {
        Ok(t) => t,
        Err(_) => {
            anyos_std::println!("fusedemo: /dev/fuse not available, exiting");
            return 1;
        }
    };
    let mut session = SessionLoop::new(transport, HelloHandler);
    match session.run() {
        Ok(()) => {
            anyos_std::println!("fusedemo: session ended cleanly");
            0
        }
        Err(_) => {
            anyos_std::println!("fusedemo: transport error");
            1
        }
    }
}

// ---------------------------------------------------------------------------
// Host-level unit tests (not run in kernel target; build-only when cargo
// test exercises the stable-host path).
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(unused_imports)]
mod tests {
    use super::*;
    #[test]
    fn lookup_hello_returns_file_attr() {
        let mut h = HelloHandler;
        match h.handle(Request::Lookup {
            parent: ROOT_INO,
            name: HELLO_NAME.to_string(),
        }) {
            HandlerResult::Ok(Reply::Lookup(a)) => {
                assert_eq!(a.ino, HELLO_INO);
                assert_eq!(a.kind, 1);
            }
            other => panic!("unexpected {:?}", other),
        }
    }
}
