// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefsd` — CoreFS FUSE-Daemon (Skelett).
//!
//! # Status: Phase 5.7 — Skeleton
//!
//! Dieser Daemon demonstriert den reinen Adapter-Pfad:
//!
//! 1. Ein `Transport` (hier: In-Memory-Mock) liefert
//!    [`corefs_fuse_proto::RequestFrame`]s.
//! 2. Ein [`FuseHandler`] interpretiert den [`Request`] und erzeugt einen
//!    [`Reply`] (bzw. einen POSIX-Errno).
//! 3. Der `SessionLoop` (aus `corefs-fuse-adapter`) verpackt das Resultat
//!    in einen [`ReplyFrame`] und schickt es via `Transport::send_reply`
//!    zurück.
//!
//! Die echte IPC-Bindung zwischen Userspace und dem AnyOS-Kernel-FUSE-
//! Subsystem (`kernel/src/fs/fuse/`) wird in einem Folgeschritt
//! nachgereicht — aktuell ist der Pfad **rein in-process** und dient
//! primär als architektonische Integrationsgarantie: der Wire-Adapter
//! kompiliert und läuft unter AnyOS-userspace.
//!
//! ## Ablaufdemo
//!
//! Der Daemon baut einen Mock-Transport mit einem einzigen `Init`-Request
//! und einem anschliessenden `Destroy`, beantwortet beide und meldet das
//! Ergebnis an stdout.

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

/// In-Memory-Transport: Requests aus einer vorgeladenen Queue, Replies
/// landen in einem Vec, den der Caller später auslesen kann.
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

/// Minimal-Handler, der Init/Destroy beantwortet und alles andere mit
/// ENOSYS quittiert. Die vollständige CoreFS-Bindung (→ `CoreFsDriver`)
/// folgt in einem späteren Kommit, sobald das FUSE-Kernel-Subsystem
/// existiert und echte Requests liefert.
struct CoreFsHandler;

const ENOSYS: i32 = 38;

impl FuseHandler for CoreFsHandler {
    fn handle(&mut self, request: Request) -> HandlerResult {
        match request {
            Request::Init { .. } => HandlerResult::Ok(Reply::Init {
                version: PROTOCOL_VERSION,
            }),
            Request::Destroy => HandlerResult::Ok(Reply::Destroy),
            _ => HandlerResult::Err {
                errno: ENOSYS,
                message: Some(String::from("corefsd: not implemented in skeleton")),
            },
        }
    }
}

fn main() -> u32 {
    anyos_std::println!("corefsd: CoreFS FUSE daemon (skeleton)");
    anyos_std::println!(
        "corefsd: protocol version {} — real kernel IPC is Phase 5.7",
        PROTOCOL_VERSION
    );

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
        Ok(()) => {
            anyos_std::println!("corefsd: session loop terminated cleanly");
        }
        Err(_) => {
            // Infallible — unreachable, but keep the branch for clarity.
            anyos_std::println!("corefsd: transport error (mock)");
            return 1;
        }
    }

    // Inspect replies for demonstration purposes.
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
        "corefsd: processed {} replies ({} ok, {} err)",
        transport.replies.len(),
        success,
        errors
    );
    0
}
