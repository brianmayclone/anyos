//! SHA-1 hashing for Git objects.
//!
//! Delegates to libtls::crypto::sha1 for the actual SHA-1 computation.

/// Compute SHA-1 of a byte slice.
pub fn hash(data: &[u8]) -> [u8; 20] {
    libtls::crypto::sha1::sha1(data)
}

/// Compute SHA-1 of a git object (with "type len\0" header).
pub fn hash_object(obj_type: &str, data: &[u8]) -> [u8; 20] {
    // Build header: "blob 1234\0"
    let mut header = alloc::vec::Vec::with_capacity(obj_type.len() + 20);
    header.extend_from_slice(obj_type.as_bytes());
    header.push(b' ');
    write_usize(&mut header, data.len());
    header.push(0);

    // Hash header + data
    let mut full = alloc::vec::Vec::with_capacity(header.len() + data.len());
    full.extend_from_slice(&header);
    full.extend_from_slice(data);
    libtls::crypto::sha1::sha1(&full)
}

fn write_usize(buf: &mut alloc::vec::Vec<u8>, mut n: usize) {
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
