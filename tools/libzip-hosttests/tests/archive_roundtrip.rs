use libzip::{crc32, deflate, gzip, tar, zip};

const WHEEZY_PACKAGES_UNPACKED_SIZE: usize = 28_480_385;

fn call_byte_api(input: &[u8], f: extern "C" fn(*const u8, u32, *mut u8, u32) -> u32) -> Vec<u8> {
    let mut probe = [0u8; 1];
    let required = f(
        input.as_ptr(),
        input.len() as u32,
        probe.as_mut_ptr(),
        probe.len() as u32,
    );
    assert_ne!(required, u32::MAX, "libzip byte API rejected input");

    let mut out = vec![0u8; required as usize];
    let written = f(
        input.as_ptr(),
        input.len() as u32,
        out.as_mut_ptr(),
        out.len() as u32,
    );
    assert_eq!(written, required);
    out
}

fn make_stored_gzip(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00]);
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    out.extend_from_slice(&[0x00, 0xff]);
    out.extend_from_slice(&deflate::store(payload));
    out.extend_from_slice(&crc32::crc32(payload).to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out
}

fn tar_entry_name(handle: u32, index: u32) -> String {
    let mut buf = [0u8; 256];
    let len = libzip::libzip_tar_entry_name(handle, index, buf.as_mut_ptr(), buf.len() as u32);
    String::from_utf8(buf[..len as usize].to_vec()).expect("tar entry name should be utf8")
}

#[test]
fn gzip_handles_debian_packages_sized_payloads() {
    let mut payload = vec![0u8; WHEEZY_PACKAGES_UNPACKED_SIZE];
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte = b'a' + (i % 26) as u8;
    }
    payload[..8].copy_from_slice(b"Package:");
    payload[WHEEZY_PACKAGES_UNPACKED_SIZE - 6..].copy_from_slice(b"zlib1g");

    let gz = make_stored_gzip(&payload);

    assert_eq!(
        gzip::gzip_decompress_with_limit(&gz, WHEEZY_PACKAGES_UNPACKED_SIZE - 1),
        Err(gzip::GZIP_ERR_TOO_LARGE)
    );

    let unpacked = gzip::gzip_decompress_with_limit(&gz, WHEEZY_PACKAGES_UNPACKED_SIZE)
        .expect("Packages-sized gzip should decompress");
    assert_eq!(unpacked.len(), WHEEZY_PACKAGES_UNPACKED_SIZE);
    assert_eq!(&unpacked[..8], b"Package:");
    assert_eq!(&unpacked[WHEEZY_PACKAGES_UNPACKED_SIZE - 6..], b"zlib1g");
}

#[test]
fn gzip_decodes_real_file_from_env() {
    let Ok(path) = std::env::var("LIBZIP_REAL_GZIP") else {
        return;
    };
    let data = std::fs::read(&path).expect("LIBZIP_REAL_GZIP should be readable");
    let unpacked =
        gzip::gzip_decompress(&data).unwrap_or_else(|| panic!("libzip failed to decode {path}"));
    assert!(unpacked.starts_with(b"Package:"));
}

#[test]
fn byte_abi_roundtrips_gzip_zlib_and_raw_deflate() {
    let input = b"licof bootstrap archive payload\n".repeat(2048);

    let gz = call_byte_api(&input, libzip::libzip_gzip_compress);
    assert_eq!(call_byte_api(&gz, libzip::libzip_gzip_decompress), input);

    let zlib = call_byte_api(&input, libzip::libzip_zlib_compress);
    assert_eq!(call_byte_api(&zlib, libzip::libzip_zlib_decompress), input);

    let raw = call_byte_api(&input, libzip::libzip_deflate_raw);
    assert_eq!(call_byte_api(&raw, libzip::libzip_inflate_raw), input);
}

#[test]
fn tar_gz_can_be_opened_from_memory_through_c_abi() {
    let mut writer = tar::TarWriter::new();
    writer.add_directory("usr/bin");
    writer.add_file("usr/bin/hello", b"#!/bin/sh\necho hello\n");
    let tar_gz = gzip::gzip_compress(&writer.finish());

    let handle = libzip::libzip_tar_open_bytes(tar_gz.as_ptr(), tar_gz.len() as u32);
    assert_ne!(handle, 0, "tar.gz bytes should produce a tar reader handle");

    assert_eq!(libzip::libzip_tar_entry_count(handle), 2);
    assert_eq!(tar_entry_name(handle, 0), "usr/bin/");
    assert_eq!(libzip::libzip_tar_entry_is_dir(handle, 0), 1);
    assert_eq!(libzip::libzip_tar_entry_typeflag(handle, 0), b'5' as u32);
    assert_eq!(libzip::libzip_tar_entry_mode(handle, 0), 0o755);

    assert_eq!(tar_entry_name(handle, 1), "usr/bin/hello");
    assert_eq!(libzip::libzip_tar_entry_size(handle, 1), 21);
    assert_eq!(libzip::libzip_tar_entry_is_dir(handle, 1), 0);
    assert_eq!(libzip::libzip_tar_entry_typeflag(handle, 1), b'0' as u32);
    assert_eq!(libzip::libzip_tar_entry_mode(handle, 1), 0o644);

    let mut out = [0u8; 64];
    let written = libzip::libzip_tar_extract(handle, 1, out.as_mut_ptr(), out.len() as u32);
    assert_eq!(written, 21);
    assert_eq!(&out[..written as usize], b"#!/bin/sh\necho hello\n");

    libzip::libzip_tar_close(handle);
}

#[test]
fn zip_writer_reader_roundtrips_stored_and_deflated_entries() {
    let mut writer = zip::ZipWriter::new();
    writer.add_directory("etc/");
    writer.add("etc/os-release", b"NAME=anyOS\nID=anyos\n", false);

    let repeated = b"bootstrap log line: dependency resolved\n".repeat(1024);
    writer.add("var/log/bootstrap.log", &repeated, true);

    let archive = writer.finish();
    let reader = zip::ZipReader::parse(archive).expect("zip should parse");

    assert_eq!(reader.entry_count(), 3);
    assert_eq!(reader.entries[0].name, "etc/");
    assert_eq!(reader.entries[1].name, "etc/os-release");
    assert_eq!(reader.entries[2].name, "var/log/bootstrap.log");
    assert_eq!(
        reader.entries[2].method, 8,
        "repeated data should be deflated"
    );

    assert_eq!(reader.extract(0).as_deref(), Some(&b""[..]));
    assert_eq!(
        reader.extract(1).as_deref(),
        Some(&b"NAME=anyOS\nID=anyos\n"[..])
    );
    assert_eq!(reader.extract(2).as_deref(), Some(repeated.as_slice()));
}
