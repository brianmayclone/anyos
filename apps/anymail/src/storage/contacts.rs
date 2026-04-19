// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Address book / contacts backed by confd with JSON import compatibility.

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;

use crate::storage::schema::schema;

/// A contact entry.
#[derive(Clone)]
pub struct Contact {
    pub name: String,
    pub email: String,
    pub notes: String,
    pub group: String,
}

impl Contact {
    pub fn new(name: &str, email: &str) -> Self {
        Self {
            name: String::from(name),
            email: String::from(email),
            notes: String::new(),
            group: String::new(),
        }
    }
}

/// The address book.
pub struct AddressBook {
    pub contacts: Vec<Contact>,
}

impl AddressBook {
    pub fn new() -> Self {
        Self {
            contacts: Vec::new(),
        }
    }

    /// Load from confd, falling back to a legacy JSON file on first run.
    pub fn load(legacy_path: &str) -> Self {
        let _ = schema().register();
        if let Some(book) = load_from_confd() {
            return book;
        }
        let book = Self::load_from_path(legacy_path);
        if !book.contacts.is_empty() {
            book.save();
        }
        book
    }

    pub fn load_from_path(path: &str) -> Self {
        let fd = anyos_std::fs::open(path, 0);
        if fd == u32::MAX {
            return Self::new();
        }

        let mut buf = alloc::vec![0u8; 64 * 1024];
        let mut total = 0usize;
        loop {
            let mut chunk = [0u8; 4096];
            let n = anyos_std::fs::read(fd, &mut chunk);
            if n == 0 || n == u32::MAX {
                break;
            }
            let n = n as usize;
            if total + n > buf.len() {
                break;
            }
            buf[total..total + n].copy_from_slice(&chunk[..n]);
            total += n;
        }
        anyos_std::fs::close(fd);

        let text = match core::str::from_utf8(&buf[..total]) {
            Ok(s) => s.trim(),
            Err(_) => return Self::new(),
        };
        Self::from_json_str(text)
    }

    /// Save the authoritative address book to confd.
    pub fn save(&self) {
        let _ = schema().register();
        let json = self.to_json_string();
        let _ = schema().write_string("config/contacts_json", &json);
    }

    pub fn save_to_path(&self, path: &str) {
        let json_str = self.to_json_string();
        let _ = anyos_std::fs::write_bytes(path, json_str.as_bytes());
    }

    fn from_json_str(text: &str) -> Self {
        let mut book = Self::new();
        if text.is_empty() {
            return book;
        }
        let json = match Value::parse(text) {
            Ok(v) => v,
            Err(_) => return book,
        };

        if let Some(arr) = json["contacts"].as_array() {
            for item in arr {
                let mut contact = Contact::new(
                    item["name"].as_str().unwrap_or(""),
                    item["email"].as_str().unwrap_or(""),
                );
                contact.notes = String::from(item["notes"].as_str().unwrap_or(""));
                contact.group = String::from(item["group"].as_str().unwrap_or(""));
                book.contacts.push(contact);
            }
        }

        book
    }

    fn to_json_string(&self) -> String {
        let mut root = Value::new_object();
        let mut arr = Value::new_array();

        for c in &self.contacts {
            let mut obj = Value::new_object();
            obj.set("name", c.name.as_str().into());
            obj.set("email", c.email.as_str().into());
            obj.set("notes", c.notes.as_str().into());
            obj.set("group", c.group.as_str().into());
            arr.push(obj);
        }

        root.set("contacts", arr);
        root.to_json_string_pretty()
    }

    /// Add a contact (avoids duplicates by email).
    pub fn add(&mut self, contact: Contact) {
        // Update existing or add new
        for c in &mut self.contacts {
            if c.email == contact.email {
                c.name = contact.name;
                c.notes = contact.notes;
                c.group = contact.group;
                return;
            }
        }
        self.contacts.push(contact);
    }

    /// Remove a contact by email.
    pub fn remove(&mut self, email: &str) {
        self.contacts.retain(|c| c.email != email);
    }

    /// Search contacts by name or email (case-insensitive substring match).
    pub fn search(&self, query: &str) -> Vec<&Contact> {
        if query.is_empty() {
            return self.contacts.iter().collect();
        }

        let q = to_lower(query);
        self.contacts
            .iter()
            .filter(|c| to_lower(&c.name).contains(&q) || to_lower(&c.email).contains(&q))
            .collect()
    }

    /// Auto-learn contacts from sent mail headers.
    pub fn learn_from_addresses(&mut self, addresses: &[crate::mail::rfc2822::EmailAddress]) {
        for addr in addresses {
            if !addr.address.is_empty() {
                let exists = self.contacts.iter().any(|c| c.email == addr.address);
                if !exists {
                    self.contacts.push(Contact {
                        name: addr.name.clone(),
                        email: addr.address.clone(),
                        notes: String::new(),
                        group: String::from("Auto"),
                    });
                }
            }
        }
    }
}

fn load_from_confd() -> Option<AddressBook> {
    let json = schema().read_string("config/contacts_json")?;
    Some(AddressBook::from_json_str(&json))
}

fn to_lower(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    for c in s.chars() {
        if c >= 'A' && c <= 'Z' {
            r.push((c as u8 + 32) as char);
        } else {
            r.push(c);
        }
    }
    r
}
