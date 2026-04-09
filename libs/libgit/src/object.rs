//! Git object types (blob, tree, commit, tag).
//!
//! Git objects are stored as: "type size\0content", zlib-compressed.

use alloc::string::String;
use alloc::vec::Vec;
use crate::oid::Oid;
use crate::sha1;

/// Git object type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Blob,
    Tree,
    Commit,
    Tag,
}

impl ObjectType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectType::Blob => "blob",
            ObjectType::Tree => "tree",
            ObjectType::Commit => "commit",
            ObjectType::Tag => "tag",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "blob" => Some(ObjectType::Blob),
            "tree" => Some(ObjectType::Tree),
            "commit" => Some(ObjectType::Commit),
            "tag" => Some(ObjectType::Tag),
            _ => None,
        }
    }
}

/// A git object with its type and content.
#[derive(Debug, Clone)]
pub struct Object {
    pub obj_type: ObjectType,
    pub data: Vec<u8>,
}

impl Object {
    /// Create a new blob object.
    pub fn blob(data: Vec<u8>) -> Self {
        Object { obj_type: ObjectType::Blob, data }
    }

    /// Create a new tree object.
    pub fn tree(data: Vec<u8>) -> Self {
        Object { obj_type: ObjectType::Tree, data }
    }

    /// Create a new commit object.
    pub fn commit(data: Vec<u8>) -> Self {
        Object { obj_type: ObjectType::Commit, data }
    }

    /// Compute the OID of this object.
    pub fn id(&self) -> Oid {
        Oid::from_bytes(sha1::hash_object(self.obj_type.as_str(), &self.data))
    }

    /// Serialize to the raw format: "type size\0content"
    pub fn serialize(&self) -> Vec<u8> {
        let type_str = self.obj_type.as_str();
        let mut buf = Vec::with_capacity(type_str.len() + 20 + self.data.len());
        buf.extend_from_slice(type_str.as_bytes());
        buf.push(b' ');
        write_usize(&mut buf, self.data.len());
        buf.push(0);
        buf.extend_from_slice(&self.data);
        buf
    }

    /// Parse from the raw format.
    pub fn deserialize(raw: &[u8]) -> Option<Self> {
        let null_pos = raw.iter().position(|&b| b == 0)?;
        let header = core::str::from_utf8(&raw[..null_pos]).ok()?;
        let space = header.find(' ')?;
        let type_str = &header[..space];
        let _size_str = &header[space + 1..];
        let obj_type = ObjectType::from_str(type_str)?;
        let data = raw[null_pos + 1..].to_vec();
        Some(Object { obj_type, data })
    }
}

/// A parsed commit.
#[derive(Debug, Clone)]
pub struct Commit {
    pub tree: Oid,
    pub parents: Vec<Oid>,
    pub author: String,
    pub committer: String,
    pub message: String,
}

impl Commit {
    /// Parse a commit object's data.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let text = core::str::from_utf8(data).ok()?;
        let mut tree = Oid::ZERO;
        let mut parents = Vec::new();
        let mut author = String::new();
        let mut committer = String::new();

        let mut lines = text.splitn(2, "\n\n");
        let header = lines.next()?;
        let message = lines.next().unwrap_or("").trim_end().into();

        for line in header.lines() {
            if let Some(rest) = line.strip_prefix("tree ") {
                tree = Oid::from_hex(rest.trim())?;
            } else if let Some(rest) = line.strip_prefix("parent ") {
                parents.push(Oid::from_hex(rest.trim())?);
            } else if let Some(rest) = line.strip_prefix("author ") {
                author = String::from(rest);
            } else if let Some(rest) = line.strip_prefix("committer ") {
                committer = String::from(rest);
            }
        }

        Some(Commit { tree, parents, author, committer, message })
    }

    /// Serialize to commit data bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = String::new();
        buf.push_str("tree ");
        buf.push_str(&self.tree.to_hex());
        buf.push('\n');
        for parent in &self.parents {
            buf.push_str("parent ");
            buf.push_str(&parent.to_hex());
            buf.push('\n');
        }
        buf.push_str("author ");
        buf.push_str(&self.author);
        buf.push('\n');
        buf.push_str("committer ");
        buf.push_str(&self.committer);
        buf.push_str("\n\n");
        buf.push_str(&self.message);
        buf.push('\n');
        buf.into_bytes()
    }
}

fn write_usize(buf: &mut Vec<u8>, mut n: usize) {
    if n == 0 {
        buf.push(b'0');
        return;
    }
    let start = buf.len();
    while n > 0 {
        buf.push(b'0' + (n % 10) as u8);
        n /= 10;
    }
    buf[start..].reverse();
}
