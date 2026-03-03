//! Address book / contacts (JSON persistence).

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;

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
        Self { contacts: Vec::new() }
    }

    /// Load contacts from JSON file.
    pub fn load(path: &str) -> Self {
        let mut book = Self::new();

        let fd = anyos_std::fs::open(path, 0);
        if fd == u32::MAX { return book; }

        let mut buf = alloc::vec![0u8; 64 * 1024];
        let mut total = 0usize;
        loop {
            let mut chunk = [0u8; 4096];
            let n = anyos_std::fs::read(fd, &mut chunk);
            if n == 0 || n == u32::MAX { break; }
            let n = n as usize;
            if total + n > buf.len() { break; }
            buf[total..total + n].copy_from_slice(&chunk[..n]);
            total += n;
        }
        anyos_std::fs::close(fd);

        if total == 0 { return book; }

        let text = match core::str::from_utf8(&buf[..total]) {
            Ok(s) => s,
            Err(_) => return book,
        };

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

    /// Save contacts to JSON file.
    pub fn save(&self, path: &str) {
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
        let json_str = root.to_json_string_pretty();
        let _ = anyos_std::fs::write_bytes(path, json_str.as_bytes());
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
        self.contacts.iter().filter(|c| {
            to_lower(&c.name).contains(&q) || to_lower(&c.email).contains(&q)
        }).collect()
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
