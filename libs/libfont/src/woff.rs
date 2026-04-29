//! WOFF 1.0 to sfnt converter.

use alloc::vec::Vec;

use crate::inflate;

struct Table {
    tag: u32,
    checksum: u32,
    orig_len: usize,
    data: Vec<u8>,
}

pub fn convert_to_sfnt(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 44 || read_u32(data, 0) != 0x774F4646 {
        return None;
    }
    let flavor = read_u32(data, 4);
    let num_tables = read_u16(data, 12) as usize;
    if num_tables == 0 {
        return None;
    }
    let dir_start = 44usize;
    let dir_len = num_tables.checked_mul(20)?;
    if data.len() < dir_start.checked_add(dir_len)? {
        return None;
    }

    let mut tables = Vec::with_capacity(num_tables);
    for i in 0..num_tables {
        let rec = dir_start + i * 20;
        let tag = read_u32(data, rec);
        let offset = read_u32(data, rec + 4) as usize;
        let comp_len = read_u32(data, rec + 8) as usize;
        let orig_len = read_u32(data, rec + 12) as usize;
        let checksum = read_u32(data, rec + 16);
        let end = offset.checked_add(comp_len)?;
        let src = data.get(offset..end)?;
        let table_data = if comp_len == orig_len {
            src.to_vec()
        } else {
            let decoded = inflate::zlib_decompress(src)?;
            if decoded.len() != orig_len {
                return None;
            }
            decoded
        };
        tables.push(Table { tag, checksum, orig_len, data: table_data });
    }

    let entry_selector = floor_log2(num_tables as u32);
    let search_range = (1u32 << entry_selector) * 16;
    let range_shift = (num_tables as u32 * 16).saturating_sub(search_range);

    let mut sfnt = Vec::new();
    write_u32(&mut sfnt, flavor);
    write_u16(&mut sfnt, num_tables as u16);
    write_u16(&mut sfnt, search_range as u16);
    write_u16(&mut sfnt, entry_selector as u16);
    write_u16(&mut sfnt, range_shift as u16);

    let dir_pos = sfnt.len();
    sfnt.resize(dir_pos + num_tables * 16, 0);

    for (i, table) in tables.iter().enumerate() {
        while sfnt.len() & 3 != 0 {
            sfnt.push(0);
        }
        let offset = sfnt.len() as u32;
        sfnt.extend_from_slice(&table.data);
        while sfnt.len() & 3 != 0 {
            sfnt.push(0);
        }

        let rec = dir_pos + i * 16;
        sfnt[rec..rec + 4].copy_from_slice(&table.tag.to_be_bytes());
        sfnt[rec + 4..rec + 8].copy_from_slice(&table.checksum.to_be_bytes());
        sfnt[rec + 8..rec + 12].copy_from_slice(&offset.to_be_bytes());
        sfnt[rec + 12..rec + 16].copy_from_slice(&(table.orig_len as u32).to_be_bytes());
    }

    Some(sfnt)
}

fn floor_log2(mut v: u32) -> u32 {
    let mut out = 0;
    while v > 1 {
        v >>= 1;
        out += 1;
    }
    out
}

fn read_u16(data: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([data[off], data[off + 1]])
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn write_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}
