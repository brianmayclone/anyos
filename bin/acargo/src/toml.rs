//! Minimal TOML parser for Cargo.toml files.
//!
//! Supports: strings, integers, booleans, tables, inline tables,
//! arrays, array-of-tables ([[section]]), dotted keys, comments.

use crate::prelude::*;
use anyos_std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Value {
    String(String),
    Integer(i64),
    Bool(bool),
    Table(Table),
    Array(Vec<Value>),
}

pub type Table = HashMap<String, Value>;

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        if let Value::String(s) = self {
            Some(s)
        } else {
            None
        }
    }
    pub fn as_table(&self) -> Option<&Table> {
        if let Value::Table(t) = self {
            Some(t)
        } else {
            None
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        if let Value::Integer(n) = self {
            Some(*n)
        } else {
            None
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        if let Value::Array(a) = self {
            Some(a)
        } else {
            None
        }
    }
}

pub fn parse(input: &str) -> Table {
    let mut root = Table::new();
    let mut current_path: Vec<String> = Vec::new();
    let mut current_is_array = false;
    let mut lines = input.lines();

    while let Some(raw_line) = lines.next() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Array of tables: [[section]]
        if line.starts_with("[[") && line.ends_with("]]") {
            let key = line[2..line.len() - 2].trim();
            current_path = split_dotted_key(key);
            current_is_array = true;
            ensure_array_table(&mut root, &current_path);
            continue;
        }

        // Table header: [section]
        if line.starts_with('[') && line.ends_with(']') {
            let key = line[1..line.len() - 1].trim();
            current_path = split_dotted_key(key);
            current_is_array = false;
            ensure_table(&mut root, &current_path);
            continue;
        }

        // Key = value
        if let Some(eq_pos) = find_equals(line) {
            let key = line[..eq_pos].trim();
            let mut val_str = strip_inline_comment(line[eq_pos + 1..].trim());
            if starts_unclosed_array(&val_str) {
                for next_line in lines.by_ref() {
                    let part = strip_inline_comment(next_line.trim());
                    if !part.is_empty() {
                        if !val_str.is_empty() {
                            val_str.push(' ');
                        }
                        val_str.push_str(&part);
                    }
                    if array_is_closed(&val_str) {
                        break;
                    }
                }
            }
            let value = parse_value(&val_str);

            if current_is_array {
                if let Some(arr_table) = get_last_array_table_mut(&mut root, &current_path) {
                    arr_table.insert(String::from(key), value);
                }
            } else if current_path.is_empty() {
                root.insert(String::from(key), value);
            } else {
                if let Some(table) = get_table_mut(&mut root, &current_path) {
                    table.insert(String::from(key), value);
                }
            }
        }
    }

    root
}

fn split_dotted_key(key: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in key.chars() {
        match quote {
            Some(q) if ch == q => {
                quote = None;
            }
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
            }
            None if ch == '.' => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() || key.ends_with('.') {
        parts.push(current.trim().to_string());
    }
    parts
}

fn starts_unclosed_array(s: &str) -> bool {
    s.trim_start().starts_with('[') && !array_is_closed(s)
}

fn array_is_closed(s: &str) -> bool {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for ch in s.chars() {
        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn find_equals(line: &str) -> Option<usize> {
    let mut in_string = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '=' if !in_string => return Some(i),
            _ => {}
        }
    }
    None
}

fn strip_inline_comment(s: &str) -> String {
    // Strip # comment but not inside strings
    let mut in_string = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '#' if !in_string => return String::from(s[..i].trim()),
            _ => {}
        }
    }
    String::from(s)
}

fn parse_value(s: &str) -> Value {
    let s = s.trim();
    // Quoted string
    if s.starts_with('"') && s.len() >= 2 {
        if let Some(end) = s[1..].find('"') {
            return Value::String(unescape(&s[1..1 + end]));
        }
    }
    // Boolean
    if s == "true" {
        return Value::Bool(true);
    }
    if s == "false" {
        return Value::Bool(false);
    }
    // Integer
    if let Some(n) = parse_i64(s) {
        return Value::Integer(n);
    }
    // Inline table: { ... }
    if s.starts_with('{') && s.ends_with('}') {
        let inner = s[1..s.len() - 1].trim();
        let mut table = Table::new();
        if !inner.is_empty() {
            for pair in split_respecting_delimiters(inner, b',') {
                let pair = pair.trim();
                if let Some(eq) = pair.find('=') {
                    let k = pair[..eq].trim();
                    let v = pair[eq + 1..].trim();
                    table.insert(String::from(k), parse_value(v));
                }
            }
        }
        return Value::Table(table);
    }
    // Array: [ ... ]
    if s.starts_with('[') && s.ends_with(']') {
        let inner = s[1..s.len() - 1].trim();
        let mut arr = Vec::new();
        if !inner.is_empty() {
            for item in split_respecting_delimiters(inner, b',') {
                let item = item.trim();
                if !item.is_empty() {
                    arr.push(parse_value(item));
                }
            }
        }
        return Value::Array(arr);
    }
    // Fallback: bare string
    Value::String(String::from(s))
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_i64(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (neg, digits) = if s.starts_with('-') {
        (true, &s[1..])
    } else {
        (false, s)
    };
    let mut n: i64 = 0;
    for b in digits.bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as i64)?;
    }
    Some(if neg { -n } else { n })
}

fn split_respecting_delimiters(s: &str, sep: u8) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut start = 0;
    for (i, b) in s.bytes().enumerate() {
        if b == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        match b {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            _ if b == sep && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

fn ensure_table(root: &mut Table, path: &[String]) {
    let mut current = root as *mut Table;
    for key in path {
        let table = unsafe { &mut *current };
        if !table.contains_key(key) {
            table.insert(key.clone(), Value::Table(Table::new()));
        }
        if let Some(Value::Table(sub)) = unsafe { &mut *current }.get_mut(key) {
            current = sub as *mut Table;
        }
    }
}

fn ensure_array_table(root: &mut Table, path: &[String]) {
    if path.is_empty() {
        return;
    }
    let last = &path[path.len() - 1];
    let parent_path = &path[..path.len() - 1];

    if !parent_path.is_empty() {
        ensure_table(root, parent_path);
    }

    let parent = if parent_path.is_empty() {
        root
    } else {
        match get_table_mut(root, parent_path) {
            Some(t) => t,
            None => return,
        }
    };

    if !parent.contains_key(last) {
        parent.insert(last.clone(), Value::Array(Vec::new()));
    }
    if let Some(Value::Array(arr)) = parent.get_mut(last) {
        arr.push(Value::Table(Table::new()));
    }
}

pub fn get_table_mut<'a>(root: &'a mut Table, path: &[String]) -> Option<&'a mut Table> {
    let mut current = root as *mut Table;
    for key in path {
        let table = unsafe { &mut *current };
        if let Some(Value::Table(sub)) = table.get_mut(key) {
            current = sub as *mut Table;
        } else {
            return None;
        }
    }
    Some(unsafe { &mut *current })
}

fn get_last_array_table_mut<'a>(root: &'a mut Table, path: &[String]) -> Option<&'a mut Table> {
    if path.is_empty() {
        return None;
    }
    let last = &path[path.len() - 1];
    let parent_path = &path[..path.len() - 1];

    let parent = if parent_path.is_empty() {
        root
    } else {
        get_table_mut(root, parent_path)?
    };

    if let Some(Value::Array(arr)) = parent.get_mut(last) {
        if let Some(Value::Table(t)) = arr.last_mut() {
            return Some(t);
        }
    }
    None
}
