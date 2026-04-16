// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//
// Kernel-side CoreFS characterization tests.
//
// These tests pin the observable behaviour of the AnyOS CoreFsDriver
// (write, flush, remount, truncate, partial-overwrite semantics).
// They run under `cargo test -p anyos_kernel` using the host target
// (not the bare-metal x86_64-anyos target).
//
// Tests marked `#[ignore]` characterize known FAILURE MODES of the
// current in-memory implementation.  They are expected to start passing
// after the extent-based refactor and must NEVER be removed — only
// un-ignored once the refactor makes them green.

#![cfg(any(test, feature = "kunit"))]

#[cfg(test)]
mod char_tests {
    use super::super::driver::{CoreFsDriver, empty_persisted_state};
    use super::super::block_device::MemSectorIo;
    use corefs_core::domain::inode::{Inode, InodeId, InodeKind};
    use corefs_core::domain::metadata::FileMetadata;
    use corefs_core::platform::Timestamp;
    use corefs_core::storage::ondisk::native::{load_state_native, save_state_native};
    use corefs_core::storage::ondisk::volume::{FormatOptions, format_device};
    use crate::fs::corefs::block_device::BlockDeviceAdapter;
    use crate::fs::file::FileType;
    use alloc::boxed::Box;
    use alloc::string::String;

    // -----------------------------------------------------------------------
    // Test infrastructure
    // -----------------------------------------------------------------------

    fn build_adapter(part_sectors: u64) -> BlockDeviceAdapter {
        let io = Box::new(MemSectorIo::new(part_sectors + 16));
        let mut adapter = BlockDeviceAdapter::with_io(0, 8, part_sectors, false, io).unwrap();
        let opts = FormatOptions {
            label: String::from("char"),
            uuid: *b"CHARTEST--------",
            inode_count: 256,
            journal_blocks: 8,
        };
        format_device(&mut adapter, &opts).expect("format");
        save_state_native(&mut adapter, &empty_persisted_state()).expect("init state");
        adapter
    }

    /// Mount, seed a root directory inode, and return the root inode number.
    fn mount_with_root(adapter: BlockDeviceAdapter) -> (CoreFsDriver, u32) {
        let driver = CoreFsDriver::mount_writable(adapter).expect("mount_writable");
        {
            let mut inner = driver.inner.lock();
            let id = InodeId(inner.next_id);
            inner.next_id += 1;
            inner.state.active_inodes.push(Inode::new_at(
                id,
                InodeKind::Directory,
                String::from("/"),
                FileMetadata::default(),
                Timestamp::EPOCH,
            ));
        }
        let root = driver.lookup("/").expect("root lookup").0;
        (driver, root)
    }

    fn prng_bytes(seed: u64, len: usize) -> alloc::vec::Vec<u8> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) as u8
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Characterization tests
    // -----------------------------------------------------------------------

    #[test]
    fn char_kernel_driver_write_1mib_roundtrips_through_flush_and_reload() {
        // Write 1 MiB, flush to device, reload state, verify bytes preserved.
        let adapter = build_adapter(4096);
        let (driver, root) = mount_with_root(adapter);
        let f = driver.create(root, "big.bin", FileType::Regular).unwrap();

        let payload = prng_bytes(0xABCD_1234, 1024 * 1024);
        // Write in 64 KiB chunks to match dd-style usage.
        let chunk_size = 64 * 1024;
        let mut written = 0usize;
        for chunk in payload.chunks(chunk_size) {
            driver.write(f, written as u32, chunk).unwrap();
            written += chunk.len();
        }
        driver.flush().expect("flush");

        // Reload state from the device (same adapter, locked inside driver).
        let inner = driver.inner.lock();
        let reloaded = load_state_native(&inner.device).expect("reload");
        let rec = reloaded
            .block_records
            .iter()
            .find(|r| {
                reloaded
                    .active_inodes
                    .iter()
                    .any(|i| i.id == r.inode && i.path == "/big.bin")
            })
            .expect("block record for /big.bin");
        assert_eq!(rec.bytes.len(), payload.len(), "1 MiB must survive flush+reload");
        assert_eq!(rec.bytes, payload, "content must be byte-for-byte identical after flush+reload");
    }

    /// Characterizes the PRE-REFACTOR failure mode: writing 128 MiB to a
    /// CoreFS instance backed by MemSectorIo (no heap limit in tests) should
    /// work at the API level.  On AnyOS with a 292 MiB heap this would OOM;
    /// the test exercises the API contract without a constrained allocator.
    #[test]
    fn char_kernel_driver_128mib_write_does_not_fail_in_test_env() {
        let adapter = build_adapter(65536); // 128 MiB-capable device
        let (driver, root) = mount_with_root(adapter);
        let f = driver.create(root, "huge.bin", FileType::Regular).unwrap();
        let chunk = alloc::vec![0xAAu8; 64 * 1024];
        let target = 128 * 1024 * 1024usize;
        let mut written = 0usize;
        while written < target {
            let n = driver.write(f, written as u32, &chunk).unwrap();
            written += n;
        }
        let (_, _, size) = driver.lookup("/huge.bin").unwrap();
        assert_eq!(size as usize, target, "reported size must match written bytes");
    }

    #[test]
    fn char_kernel_driver_truncate_extends_with_zero_fill_and_survives_flush() {
        let adapter = build_adapter(4096);
        let (driver, root) = mount_with_root(adapter);
        let f = driver.create(root, "t.bin", FileType::Regular).unwrap();
        driver.write(f, 0, b"AB").unwrap();
        // Extend to 6 bytes via truncate — tail must be zero-filled.
        driver.truncate_file(f, 6).unwrap();
        let mut buf = [0xFFu8; 8];
        let n = driver.read(f, 0, &mut buf).unwrap();
        assert_eq!(n, 6);
        assert_eq!(&buf[..6], b"AB\x00\x00\x00\x00");

        // Flush + reload — zero-fill must survive persistence.
        driver.flush().expect("flush");
        let inner = driver.inner.lock();
        let reloaded = load_state_native(&inner.device).expect("reload");
        let rec = reloaded.block_records.iter().find(|r| {
            reloaded.active_inodes.iter().any(|i| i.id == r.inode && i.path == "/t.bin")
        }).expect("block record for /t.bin");
        assert_eq!(rec.bytes.len(), 6);
        assert_eq!(&rec.bytes[..], b"AB\x00\x00\x00\x00");
    }

    #[test]
    fn char_kernel_driver_partial_write_preserves_surrounding_bytes() {
        let adapter = build_adapter(4096);
        let (driver, root) = mount_with_root(adapter);
        let f = driver.create(root, "patch.bin", FileType::Regular).unwrap();

        // Initial content: 10 KiB of known bytes.
        let initial: alloc::vec::Vec<u8> = (0..10 * 1024).map(|i| (i & 0xFF) as u8).collect();
        driver.write(f, 0, &initial).unwrap();

        // Overwrite bytes [3000..3100] with a distinct pattern.
        let patch = alloc::vec![0xFFu8; 100];
        driver.write(f, 3000, &patch).unwrap();

        // Read back the full file.
        let mut buf = alloc::vec![0u8; 10 * 1024];
        let n = driver.read(f, 0, &mut buf).unwrap();
        assert_eq!(n, 10 * 1024);
        // Bytes before patch.
        assert_eq!(&buf[..3000], &initial[..3000]);
        // Patched region.
        assert_eq!(&buf[3000..3100], patch.as_slice());
        // Bytes after patch.
        assert_eq!(&buf[3100..], &initial[3100..]);
    }

    #[test]
    fn char_kernel_driver_write_read_small_file_after_flush_and_reload() {
        // Basic write→flush→reload roundtrip for a small file.
        let adapter = build_adapter(4096);
        let (driver, root) = mount_with_root(adapter);
        let f = driver.create(root, "small.txt", FileType::Regular).unwrap();
        driver.write(f, 0, b"small payload").unwrap();
        driver.flush().expect("flush");

        let inner = driver.inner.lock();
        let reloaded = load_state_native(&inner.device).expect("reload");
        assert!(
            reloaded.block_records.iter().any(|r| r.bytes == b"small payload"),
            "small payload must survive flush+reload"
        );
    }
}
