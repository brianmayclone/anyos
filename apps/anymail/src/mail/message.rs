// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Email message data model.

use super::rfc2822::EmailAddress;
use alloc::string::String;
use alloc::vec::Vec;

/// Flags on a message (bitfield).
pub const FLAG_SEEN: u32 = 1;
pub const FLAG_ANSWERED: u32 = 2;
pub const FLAG_FLAGGED: u32 = 4;
pub const FLAG_DELETED: u32 = 8;
pub const FLAG_DRAFT: u32 = 16;
pub const FLAG_JUNK: u32 = 32;
pub const FLAG_HAS_ATTACHMENT: u32 = 64;

/// Summary of a message (stored in index, displayed in grid).
#[derive(Clone)]
pub struct MessageSummary {
    pub uid: u32,
    pub message_id: String,
    pub from: EmailAddress,
    pub to: Vec<EmailAddress>,
    pub subject: String,
    pub date: String, // Sortable: YYYY-MM-DD HH:MM:SS
    pub size: u64,
    pub flags: u32,
    pub in_reply_to: String,
    pub references: String,
    pub preview: String, // First ~100 chars of plain text body
}

impl MessageSummary {
    pub fn new() -> Self {
        Self {
            uid: 0,
            message_id: String::new(),
            from: EmailAddress::new(""),
            to: Vec::new(),
            subject: String::new(),
            date: String::new(),
            size: 0,
            flags: 0,
            in_reply_to: String::new(),
            references: String::new(),
            preview: String::new(),
        }
    }

    pub fn is_seen(&self) -> bool {
        self.flags & FLAG_SEEN != 0
    }
    pub fn is_flagged(&self) -> bool {
        self.flags & FLAG_FLAGGED != 0
    }
    pub fn is_answered(&self) -> bool {
        self.flags & FLAG_ANSWERED != 0
    }
    pub fn is_deleted(&self) -> bool {
        self.flags & FLAG_DELETED != 0
    }
    pub fn is_draft(&self) -> bool {
        self.flags & FLAG_DRAFT != 0
    }
    pub fn is_junk(&self) -> bool {
        self.flags & FLAG_JUNK != 0
    }
    pub fn has_attachment(&self) -> bool {
        self.flags & FLAG_HAS_ATTACHMENT != 0
    }
}

/// A parsed attachment.
#[derive(Clone)]
pub struct Attachment {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
    pub size: u64,
    pub content_id: String, // For inline images (cid:xxx)
}

/// A fully parsed message with body parts.
pub struct FullMessage {
    pub summary: MessageSummary,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    pub text_body: String, // Plain text body
    pub html_body: String, // HTML body (may be empty)
    pub attachments: Vec<Attachment>,
    pub raw: Vec<u8>, // Raw RFC 2822 source
}

impl FullMessage {
    pub fn new() -> Self {
        Self {
            summary: MessageSummary::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            text_body: String::new(),
            html_body: String::new(),
            attachments: Vec::new(),
            raw: Vec::new(),
        }
    }

    /// Get the best body for display (HTML preferred, text as fallback).
    pub fn display_body(&self) -> &str {
        if !self.html_body.is_empty() {
            &self.html_body
        } else {
            &self.text_body
        }
    }

    /// Check if this is an HTML email.
    pub fn is_html(&self) -> bool {
        !self.html_body.is_empty()
    }
}

/// Mode for composing a message.
#[derive(Clone, Copy, PartialEq)]
pub enum ComposeMode {
    New,
    Reply,
    ReplyAll,
    Forward,
    Draft,
}
