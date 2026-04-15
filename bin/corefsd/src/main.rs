// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefsd` — CoreFS FUSE-Daemon.
//!
//! # Status: Phase 5.7 — Echte `/dev/fuse`-IPC (Skelett)
//!
//! Der Daemon öffnet `/dev/fuse` via `anyos_std::fs::open` und führt eine
//! einfache Event-Loop:
//!
//! 1. `fs::read(fd, buf)` → liest den nächsten RequestFrame als
//!    `unique (8 LE bytes) || opake body`.
//! 2. Wenn der Body leer ist (`n == 8`), wartet der Daemon kurz und pollt
//!    erneut (der Kernel-Adapter liefert `n = 0`, wenn die Queue leer ist).
//!    Der aktuelle `/dev/fuse`-Pfad ist non-blocking.
//! 3. `bincode`-Deserialisierung des Body → [`RequestFrame`], Handler
//!    liefert [`Reply`], Wire-Reply wird als `unique || body` rausgeschrieben.
//!
//! # Scope der Handler-Anbindung
//!
//! Die tatsächliche CoreFS-Bindung (`OdfDeviceSession`-Ownership, Inode-
//! Mapping, POSIX-Pfad) ist nicht Teil dieses Commits — `CoreFsHandler`
//! beantwortet aktuell nur `Init` + `Destroy` korrekt, alles andere liefert
//! `ENOSYS`. Der IPC-Pfad ist damit testbar, die fachliche Vollständigkeit
//! bleibt Folgecommits überlassen.
//!
//! Das Fallback-In-Memory-Mode (`--demo`) steht weiterhin zur Verfügung, um
//! den Wire-Adapter ohne laufenden Kernel-FUSE-Stack zu validieren.

#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::convert::Infallible;

use corefs_fuse_adapter::{FuseHandler, HandlerResult, SessionLoop, Transport};
use corefs_fuse_proto::{
    FrameHeader, PROTOCOL_VERSION, Reply, ReplyFrame, ReplyPayload, Request, RequestFrame,
};

anyos_std::entry!(main);

const ENOSYS: i32 = 38;
const DEV_FUSE: &str = "/dev/fuse";
const MAX_FRAME: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Mock-Transport (Demo-Modus — reiner In-Process-Pfad)
// ---------------------------------------------------------------------------

struct MockTransport {
    requests: Vec<RequestFrame>,
    replies: Vec<ReplyFrame>,
    cursor: usize,
}

impl MockTransport {
    fn with_requests(requests: Vec<RequestFrame>) -> Self {
        Self {
            requests,
            replies: Vec::new(),
            cursor: 0,
        }
    }
}

impl Transport for MockTransport {
    type Error = Infallible;

    fn recv_request(&mut self) -> Result<Option<RequestFrame>, Self::Error> {
        if self.cursor >= self.requests.len() {
            return Ok(None);
        }
        let frame = self.requests[self.cursor].clone();
        self.cursor += 1;
        Ok(Some(frame))
    }

    fn send_reply(&mut self, reply: &ReplyFrame) -> Result<(), Self::Error> {
        self.replies.push(reply.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// DevFuseTransport — echter `/dev/fuse`-Pfad
// ---------------------------------------------------------------------------

/// Fehler auf dem `/dev/fuse`-IPC-Pfad.
#[derive(Debug, Clone, Copy)]
enum DevFuseError {
    /// `open("/dev/fuse")` schlug fehl.
    OpenFailed,
    /// Kurzer oder malformed Request-Frame (< 8 Bytes Header).
    ShortFrame,
    /// Decoding der Body-Bytes zurück auf `RequestFrame` schlug fehl.
    DecodeFailed,
    /// Encoding des ReplyFrames schlug fehl.
    EncodeFailed,
    /// Maximale Frame-Größe überschritten.
    FrameTooLarge,
    /// Write zurück auf `/dev/fuse` schlug fehl.
    WriteFailed,
}

/// Transport über das Kernel-FUSE-Char-Device `/dev/fuse`.
///
/// Wire-Format (siehe `kernel/src/fs/fuse/mod.rs`):
/// - Read liefert `unique (8 LE) || body`.
/// - Write erwartet `unique (8 LE) || body`.
struct DevFuseTransport {
    fd: u32,
    scratch: Vec<u8>,
}

impl DevFuseTransport {
    fn open() -> Result<Self, DevFuseError> {
        let fd = anyos_std::fs::open(
            DEV_FUSE,
            anyos_std::fs::O_WRITE, // RW; AnyOS uses O_WRITE bit; absent means RO
        );
        if fd == u32::MAX {
            return Err(DevFuseError::OpenFailed);
        }
        Ok(Self {
            fd,
            scratch: vec![0u8; MAX_FRAME],
        })
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

        // Decode body → Request.
        let request: Request = match decode_request(body) {
            Some(r) => r,
            None => return Err(DevFuseError::DecodeFailed),
        };
        Ok(Some(RequestFrame {
            header: FrameHeader { unique },
            op: request,
        }))
    }

    fn send_reply(&mut self, reply: &ReplyFrame) -> Result<(), Self::Error> {
        let body = match encode_reply(reply) {
            Some(b) => b,
            None => return Err(DevFuseError::EncodeFailed),
        };
        let total = 8 + body.len();
        if total > MAX_FRAME {
            return Err(DevFuseError::FrameTooLarge);
        }
        let mut frame = Vec::with_capacity(total);
        frame.extend_from_slice(&reply.header.unique.to_le_bytes());
        frame.extend_from_slice(&body);
        let n = anyos_std::fs::write(self.fd, &frame);
        if n == u32::MAX {
            return Err(DevFuseError::WriteFailed);
        }
        Ok(())
    }
}

fn decode_request(bytes: &[u8]) -> Option<Request> {
    // Wire body = bincode-encoded `Request` op only (ohne Frame-Header —
    // der `unique`-Counter fliesst als separates 8-Byte-Prefix auf dem
    // `/dev/fuse`-Kanal).
    //
    // Wir dekodieren als RequestFrame mit Dummy-Header, falls der Kernel
    // doch einmal einen kompletten Frame vorlegt; andernfalls versuchen
    // wir den op-only Pfad. Aktuell verwenden beide Seiten `op only`.
    bincode::serde::decode_from_slice::<Request, _>(
        bytes,
        bincode::config::legacy(),
    )
    .map(|(v, _)| v)
    .ok()
}

fn encode_reply(reply: &ReplyFrame) -> Option<Vec<u8>> {
    // Wire body = bincode-encoded `ReplyPayload`. Der `unique`-Header
    // wird vom Transport separat vorangestellt.
    bincode::serde::encode_to_vec(&reply.payload, bincode::config::legacy()).ok()
}

// ---------------------------------------------------------------------------
// CoreFsHandler (Skelett — echte CoreFS-Bindung in Folgecommits)
// ---------------------------------------------------------------------------

struct CoreFsHandler;

impl FuseHandler for CoreFsHandler {
    fn handle(&mut self, request: Request) -> HandlerResult {
        match request {
            Request::Init { .. } => HandlerResult::Ok(Reply::Init {
                version: PROTOCOL_VERSION,
            }),
            Request::Destroy => HandlerResult::Ok(Reply::Destroy),
            _ => HandlerResult::Err {
                errno: ENOSYS,
                message: Some(String::from("corefsd: not implemented yet")),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() -> u32 {
    anyos_std::println!("corefsd: CoreFS FUSE daemon (phase 5.7)");
    anyos_std::println!(
        "corefsd: protocol version {}, wire = /dev/fuse",
        PROTOCOL_VERSION
    );

    // Default-Modus: echter `/dev/fuse`-Pfad. Wenn `open` fehlschlägt
    // (z. B. DevFs hat `/dev/fuse` nicht registriert — sollte nach
    // Phase-5.7-Patch nicht mehr vorkommen), fallback in Demo-Mode.
    match DevFuseTransport::open() {
        Ok(transport) => {
            anyos_std::println!("corefsd: opened /dev/fuse, entering event loop");
            run_real(transport)
        }
        Err(_) => {
            anyos_std::println!(
                "corefsd: /dev/fuse not available, running built-in demo loop"
            );
            run_demo()
        }
    }
}

fn run_real(transport: DevFuseTransport) -> u32 {
    let handler = CoreFsHandler;
    let mut session = SessionLoop::new(transport, handler);
    match session.run() {
        Ok(()) => {
            anyos_std::println!("corefsd: session loop terminated cleanly");
            0
        }
        Err(_) => {
            anyos_std::println!("corefsd: transport error on /dev/fuse");
            1
        }
    }
}

fn run_demo() -> u32 {
    let requests = vec![
        RequestFrame {
            header: FrameHeader { unique: 1 },
            op: Request::Init {
                version: PROTOCOL_VERSION,
            },
        },
        RequestFrame {
            header: FrameHeader { unique: 2 },
            op: Request::Destroy,
        },
    ];
    let transport = MockTransport::with_requests(requests);
    let handler = CoreFsHandler;
    let mut session = SessionLoop::new(transport, handler);

    match session.run() {
        Ok(()) => anyos_std::println!("corefsd: demo session loop terminated cleanly"),
        Err(_) => {
            anyos_std::println!("corefsd: demo transport error (Infallible, unreachable)");
            return 1;
        }
    }

    let (transport, _handler) = session.into_parts();
    let mut success = 0usize;
    let mut errors = 0usize;
    for r in &transport.replies {
        match &r.payload {
            ReplyPayload::Ok(_) => success += 1,
            ReplyPayload::Err { .. } => errors += 1,
        }
    }
    anyos_std::println!(
        "corefsd: demo processed {} replies ({} ok, {} err)",
        transport.replies.len(),
        success,
        errors
    );
    0
}
