//! NTFS data run decoding for non-resident attributes.
//!
//! Non-resident attribute data is described by a compact variable-length
//! encoding of (cluster_count, lcn_offset) pairs called "data runs".

use alloc::vec::Vec;

/// A decoded data run: a contiguous range of clusters on disk.
#[derive(Debug, Clone, Copy)]
pub(super) struct DataRun {
    /// Logical Cluster Number (absolute position on disk).
    /// `None` means a sparse (zeroed) run.
    pub lcn: Option<u64>,
    /// Number of clusters in this run.
    pub length: u64,
}

/// Decode data runs from a byte slice.
///
/// The run list is a sequence of variable-length entries:
/// - Header byte: `(offset_bytes << 4) | length_bytes`
/// - length_bytes: unsigned LE → cluster count
/// - offset_bytes: **signed** LE → LCN delta relative to previous run
/// - Terminated by 0x00 byte.
pub(super) fn decode_data_runs(data: &[u8]) -> Vec<DataRun> {
    let mut runs = Vec::new();
    let mut pos = 0;
    let mut prev_lcn: i64 = 0;

    loop {
        if pos >= data.len() {
            break;
        }

        let header = data[pos];
        if header == 0 {
            break;
        }
        pos += 1;

        let length_bytes = (header & 0x0F) as usize;
        let offset_bytes = ((header >> 4) & 0x0F) as usize;

        if length_bytes == 0 || pos + length_bytes + offset_bytes > data.len() {
            break;
        }

        // Read cluster count (unsigned)
        let mut count: u64 = 0;
        for i in 0..length_bytes {
            count |= (data[pos + i] as u64) << (i * 8);
        }
        pos += length_bytes;

        // Read LCN offset (signed, delta from previous run)
        if offset_bytes == 0 {
            // Sparse run — no LCN
            runs.push(DataRun { lcn: None, length: count });
        } else {
            let mut offset: i64 = 0;
            for i in 0..offset_bytes {
                offset |= (data[pos + i] as i64) << (i * 8);
            }
            // Sign-extend if high bit is set
            if offset_bytes < 8 && (data[pos + offset_bytes - 1] & 0x80) != 0 {
                for i in offset_bytes..8 {
                    offset |= 0xFFi64 << (i * 8);
                }
            }
            pos += offset_bytes;

            prev_lcn += offset;
            if prev_lcn < 0 {
                break; // invalid
            }
            runs.push(DataRun {
                lcn: Some(prev_lcn as u64),
                length: count,
            });
        }
    }

    runs
}

/// Calculate total cluster count across all runs.
pub(super) fn total_clusters(runs: &[DataRun]) -> u64 {
    runs.iter().map(|r| r.length).sum()
}
