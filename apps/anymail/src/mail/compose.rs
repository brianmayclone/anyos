// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! MIME message composition for sending email.

use super::message::{Attachment, ComposeMode, FullMessage};
use super::rfc2822::EmailAddress;
use super::{base64, rfc2822};
use alloc::string::String;
use alloc::vec::Vec;

/// Build an RFC 2822 MIME message ready for SMTP DATA.
pub fn build_message(
    from: &EmailAddress,
    to: &[EmailAddress],
    cc: &[EmailAddress],
    subject: &str,
    text_body: &str,
    html_body: &str,
    attachments: &[Attachment],
    in_reply_to: &str,
    references: &str,
) -> Vec<u8> {
    let mut msg = String::new();

    // Generate a simple message ID
    let msg_id = generate_message_id(&from.address);

    // Headers
    msg.push_str("From: ");
    msg.push_str(&from.to_header_string());
    msg.push_str("\r\n");

    msg.push_str("To: ");
    msg.push_str(&format_address_list(to));
    msg.push_str("\r\n");

    if !cc.is_empty() {
        msg.push_str("Cc: ");
        msg.push_str(&format_address_list(cc));
        msg.push_str("\r\n");
    }

    msg.push_str("Subject: ");
    msg.push_str(subject);
    msg.push_str("\r\n");

    msg.push_str("Date: ");
    msg.push_str(&rfc2822_date());
    msg.push_str("\r\n");

    msg.push_str("Message-ID: <");
    msg.push_str(&msg_id);
    msg.push_str(">\r\n");

    if !in_reply_to.is_empty() {
        msg.push_str("In-Reply-To: ");
        msg.push_str(in_reply_to);
        msg.push_str("\r\n");
    }

    if !references.is_empty() {
        msg.push_str("References: ");
        msg.push_str(references);
        msg.push_str("\r\n");
    }

    msg.push_str("MIME-Version: 1.0\r\n");
    msg.push_str("User-Agent: anyMail/1.0\r\n");

    let has_html = !html_body.is_empty();
    let has_attachments = !attachments.is_empty();

    if has_attachments {
        // multipart/mixed (attachments + body)
        let boundary_mixed = alloc::format!("anymail-mixed-{}", msg_id_counter());
        msg.push_str("Content-Type: multipart/mixed;\r\n boundary=\"");
        msg.push_str(&boundary_mixed);
        msg.push_str("\"\r\n\r\n");
        msg.push_str("This is a multi-part message in MIME format.\r\n\r\n");

        // Body part
        msg.push_str("--");
        msg.push_str(&boundary_mixed);
        msg.push_str("\r\n");

        if has_html {
            // multipart/alternative for text+html
            let boundary_alt = alloc::format!("anymail-alt-{}", msg_id_counter());
            msg.push_str("Content-Type: multipart/alternative;\r\n boundary=\"");
            msg.push_str(&boundary_alt);
            msg.push_str("\"\r\n\r\n");

            // Plain text part
            msg.push_str("--");
            msg.push_str(&boundary_alt);
            msg.push_str("\r\n");
            write_text_part(&mut msg, text_body);

            // HTML part
            msg.push_str("--");
            msg.push_str(&boundary_alt);
            msg.push_str("\r\n");
            write_html_part(&mut msg, html_body);

            msg.push_str("--");
            msg.push_str(&boundary_alt);
            msg.push_str("--\r\n");
        } else {
            write_text_part(&mut msg, text_body);
        }

        // Attachment parts
        for att in attachments {
            msg.push_str("--");
            msg.push_str(&boundary_mixed);
            msg.push_str("\r\n");
            write_attachment_part(&mut msg, att);
        }

        msg.push_str("--");
        msg.push_str(&boundary_mixed);
        msg.push_str("--\r\n");
    } else if has_html {
        // multipart/alternative (text + html, no attachments)
        let boundary = alloc::format!("anymail-alt-{}", msg_id_counter());
        msg.push_str("Content-Type: multipart/alternative;\r\n boundary=\"");
        msg.push_str(&boundary);
        msg.push_str("\"\r\n\r\n");

        msg.push_str("--");
        msg.push_str(&boundary);
        msg.push_str("\r\n");
        write_text_part(&mut msg, text_body);

        msg.push_str("--");
        msg.push_str(&boundary);
        msg.push_str("\r\n");
        write_html_part(&mut msg, html_body);

        msg.push_str("--");
        msg.push_str(&boundary);
        msg.push_str("--\r\n");
    } else {
        // Simple text/plain message
        write_text_part(&mut msg, text_body);
    }

    msg.into_bytes()
}

/// Prepare reply headers and quoted body.
pub fn prepare_reply(
    original: &FullMessage,
    reply_all: bool,
) -> (
    Vec<EmailAddress>,
    Vec<EmailAddress>,
    String,
    String,
    String,
    String,
) {
    // To
    let to = if !original.reply_to.is_empty() {
        original.reply_to.clone()
    } else {
        alloc::vec![original.summary.from.clone()]
    };

    // Cc (only for reply all)
    let cc = if reply_all {
        let mut cc_list = Vec::new();
        for addr in &original.summary.to {
            cc_list.push(addr.clone());
        }
        for addr in &original.cc {
            cc_list.push(addr.clone());
        }
        cc_list
    } else {
        Vec::new()
    };

    // Subject
    let subject = if original.summary.subject.starts_with("Re:")
        || original.summary.subject.starts_with("RE:")
    {
        original.summary.subject.clone()
    } else {
        alloc::format!("Re: {}", original.summary.subject)
    };

    // In-Reply-To
    let in_reply_to = original.summary.message_id.clone();

    // References
    let references = if original.summary.references.is_empty() {
        original.summary.message_id.clone()
    } else {
        alloc::format!(
            "{} {}",
            original.summary.references,
            original.summary.message_id
        )
    };

    // Quoted body
    let body = format_quoted_body(
        &original.summary.from.display_short(),
        &original.summary.date,
        &original.text_body,
    );

    (to, cc, subject, body, in_reply_to, references)
}

/// Prepare forward message.
pub fn prepare_forward(original: &FullMessage) -> (String, String) {
    let subject = if original.summary.subject.starts_with("Fwd:")
        || original.summary.subject.starts_with("FWD:")
    {
        original.summary.subject.clone()
    } else {
        alloc::format!("Fwd: {}", original.summary.subject)
    };

    let mut body = String::from("\r\n\r\n---------- Forwarded message ----------\r\n");
    body.push_str("From: ");
    body.push_str(&original.summary.from.to_header_string());
    body.push_str("\r\nDate: ");
    body.push_str(&original.summary.date);
    body.push_str("\r\nSubject: ");
    body.push_str(&original.summary.subject);
    body.push_str("\r\nTo: ");
    for (i, to) in original.summary.to.iter().enumerate() {
        if i > 0 {
            body.push_str(", ");
        }
        body.push_str(&to.to_header_string());
    }
    body.push_str("\r\n\r\n");
    body.push_str(&original.text_body);

    (subject, body)
}

fn write_text_part(msg: &mut String, text: &str) {
    msg.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    msg.push_str("Content-Transfer-Encoding: quoted-printable\r\n\r\n");
    let encoded = super::quoted_printable::encode(text.as_bytes());
    if let Ok(s) = core::str::from_utf8(&encoded) {
        msg.push_str(s);
    }
    msg.push_str("\r\n");
}

fn write_html_part(msg: &mut String, html: &str) {
    msg.push_str("Content-Type: text/html; charset=utf-8\r\n");
    msg.push_str("Content-Transfer-Encoding: quoted-printable\r\n\r\n");
    let encoded = super::quoted_printable::encode(html.as_bytes());
    if let Ok(s) = core::str::from_utf8(&encoded) {
        msg.push_str(s);
    }
    msg.push_str("\r\n");
}

fn write_attachment_part(msg: &mut String, att: &Attachment) {
    msg.push_str("Content-Type: ");
    msg.push_str(&att.content_type);
    msg.push_str(";\r\n name=\"");
    msg.push_str(&att.filename);
    msg.push_str("\"\r\n");
    msg.push_str("Content-Disposition: attachment;\r\n filename=\"");
    msg.push_str(&att.filename);
    msg.push_str("\"\r\n");
    msg.push_str("Content-Transfer-Encoding: base64\r\n\r\n");

    // Base64 encode with line wrapping at 76 chars
    let encoded = base64::encode(&att.data);
    let mut col = 0;
    for &b in &encoded {
        msg.push(b as char);
        col += 1;
        if col >= 76 {
            msg.push_str("\r\n");
            col = 0;
        }
    }
    if col > 0 {
        msg.push_str("\r\n");
    }
}

fn format_address_list(addrs: &[EmailAddress]) -> String {
    let mut s = String::new();
    let mut first = true;
    for addr in addrs {
        if !first {
            s.push_str(", ");
        }
        first = false;
        let h = addr.to_header_string();
        s.push_str(&h);
    }
    s
}

fn format_quoted_body(from_name: &str, date: &str, body: &str) -> String {
    let mut quoted = alloc::format!("\r\n\r\nOn {}, {} wrote:\r\n", date, from_name);
    for line in body.split('\n') {
        let line = line.trim_end_matches('\r');
        quoted.push_str("> ");
        quoted.push_str(line);
        quoted.push_str("\r\n");
    }
    quoted
}

fn generate_message_id(from_addr: &str) -> String {
    let domain = rfc2822::domain_of(from_addr);
    let counter = msg_id_counter();
    alloc::format!("{}.anymail@{}", counter, domain)
}

static mut MSG_ID_COUNTER: u32 = 1000;

fn msg_id_counter() -> u32 {
    unsafe {
        MSG_ID_COUNTER += 1;
        MSG_ID_COUNTER
    }
}

fn rfc2822_date() -> String {
    let mut buf = [0u8; 8];
    anyos_std::sys::time(&mut buf);
    let year = buf[0] as u16 | ((buf[1] as u16) << 8);
    let month = buf[2];
    let day = buf[3];
    let hour = buf[4];
    let min = buf[5];
    let sec = buf[6];
    let mon_name = match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "Jan",
    };
    alloc::format!(
        "{:02} {} {:04} {:02}:{:02}:{:02} +0000",
        day,
        mon_name,
        year,
        hour,
        min,
        sec
    )
}
