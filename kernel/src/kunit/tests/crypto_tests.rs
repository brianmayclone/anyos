//! Unit tests for kernel crypto primitives.

use crate::crypto::md5::{md5, md5_hex};
use crate::crypto::password::{hash_password, needs_rehash, verify_password};
use crate::crypto::sha256::{sha256, sha256_hex};
use crate::kunit::{TestCase, TestContext, TestSuite};

pub static SUITE: TestSuite = TestSuite {
    name: "crypto",
    cases: &[
        TestCase { name: "rfc1321_empty_string",   run: test_empty },
        TestCase { name: "rfc1321_a",              run: test_a },
        TestCase { name: "rfc1321_abc",            run: test_abc },
        TestCase { name: "rfc1321_message_digest", run: test_message_digest },
        TestCase { name: "rfc1321_a_to_z",         run: test_a_to_z },
        TestCase { name: "rfc1321_alphanumeric",   run: test_alphanumeric },
        TestCase { name: "rfc1321_8x_digits",      run: test_8x_digits },
        TestCase { name: "md5_hex_format",         run: test_hex_format },
        TestCase { name: "different_inputs_differ", run: test_differs },
        TestCase { name: "same_input_stable",      run: test_stable },
        TestCase { name: "sha256_empty_string",    run: test_sha256_empty },
        TestCase { name: "sha256_abc",             run: test_sha256_abc },
        TestCase { name: "sha256_hex_format",      run: test_sha256_hex_format },
        TestCase { name: "password_hash_roundtrip", run: test_password_roundtrip },
        TestCase { name: "password_rehash_legacy_md5", run: test_password_needs_rehash },
    ],
};

// RFC 1321 Appendix A.5 test vectors ─────────────────────────────────────────

fn test_empty(ctx: &mut TestContext) {
    let digest = md5(b"");
    ctx.expect_eq(
        digest,
        [0xd4, 0x1d, 0x8c, 0xd9, 0x8f, 0x00, 0xb2, 0x04,
         0xe9, 0x80, 0x09, 0x98, 0xec, 0xf8, 0x42, 0x7e],
        "MD5(\"\") == d41d8cd98f00b204e9800998ecf8427e"
    );
}

fn test_a(ctx: &mut TestContext) {
    let digest = md5(b"a");
    ctx.expect_eq(
        digest,
        [0x0c, 0xc1, 0x75, 0xb9, 0xc0, 0xf1, 0xb6, 0xa8,
         0x31, 0xc3, 0x99, 0xe2, 0x69, 0x77, 0x26, 0x61],
        "MD5(\"a\") == 0cc175b9c0f1b6a831c399e269772661"
    );
}

fn test_abc(ctx: &mut TestContext) {
    let digest = md5(b"abc");
    ctx.expect_eq(
        digest,
        [0x90, 0x01, 0x50, 0x98, 0x3c, 0xd2, 0x4f, 0xb0,
         0xd6, 0x96, 0x3f, 0x7d, 0x28, 0xe1, 0x7f, 0x72],
        "MD5(\"abc\") == 900150983cd24fb0d6963f7d28e17f72"
    );
}

fn test_message_digest(ctx: &mut TestContext) {
    let digest = md5(b"message digest");
    ctx.expect_eq(
        digest,
        [0xf9, 0x6b, 0x69, 0x7d, 0x7c, 0xb7, 0x93, 0x8d,
         0x52, 0x5a, 0x2f, 0x31, 0xaa, 0xf1, 0x61, 0xd0],
        "MD5(\"message digest\")"
    );
}

fn test_a_to_z(ctx: &mut TestContext) {
    let digest = md5(b"abcdefghijklmnopqrstuvwxyz");
    ctx.expect_eq(
        digest,
        [0xc3, 0xfc, 0xd3, 0xd7, 0x61, 0x92, 0xe4, 0x00,
         0x7d, 0xfb, 0x49, 0x6c, 0xca, 0x67, 0xe1, 0x3b],
        "MD5(\"abcdefghijklmnopqrstuvwxyz\")"
    );
}

fn test_alphanumeric(ctx: &mut TestContext) {
    let digest = md5(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789");
    ctx.expect_eq(
        digest,
        [0xd1, 0x74, 0xab, 0x98, 0xd2, 0x77, 0xd9, 0xf5,
         0xa5, 0x61, 0x1c, 0x2c, 0x9f, 0x41, 0x9d, 0x9f],
        "MD5(A..Za..z0..9)"
    );
}

fn test_8x_digits(ctx: &mut TestContext) {
    // Exactly 80 ASCII digits — RFC 1321 Appendix A.5 last test vector.
    // Must be on one line; a `\` continuation would produce 100 chars instead.
    let digest = md5(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890");
    ctx.expect_eq(
        digest,
        [0x57, 0xed, 0xf4, 0xa2, 0x2b, 0xe3, 0xc9, 0x55,
         0xac, 0x49, 0xda, 0x2e, 0x21, 0x07, 0xb6, 0x7a],
        "MD5(80-digit number string)"
    );
}

// Additional properties ───────────────────────────────────────────────────────

fn test_hex_format(ctx: &mut TestContext) {
    let hex = md5_hex(b"abc");
    // Expected lowercase hex of the abc digest.
    let expected = *b"900150983cd24fb0d6963f7d28e17f72";
    ctx.expect_eq(hex, expected, "md5_hex produces correct lowercase hex for \"abc\"");
    // All bytes must be valid ASCII hex digits.
    let all_hex = hex.iter().all(|&b| b.is_ascii_hexdigit());
    ctx.expect_true(all_hex, "md5_hex output is all ASCII hex digits");
}

fn test_differs(ctx: &mut TestContext) {
    let d1 = md5(b"foo");
    let d2 = md5(b"bar");
    ctx.expect_ne(d1, d2, "different inputs produce different digests");
}

fn test_stable(ctx: &mut TestContext) {
    // Same input must always produce the same output.
    let d1 = md5(b"anyOS kernel test");
    let d2 = md5(b"anyOS kernel test");
    ctx.expect_eq(d1, d2, "same input always produces same digest");
}

fn test_sha256_empty(ctx: &mut TestContext) {
    let digest = sha256(b"");
    ctx.expect_eq(
        digest,
        [
            0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14,
            0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24,
            0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c,
            0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55,
        ],
        "SHA-256(\"\") matches FIPS test vector",
    );
}

fn test_sha256_abc(ctx: &mut TestContext) {
    let digest = sha256(b"abc");
    ctx.expect_eq(
        digest,
        [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea,
            0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
            0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
            0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
        ],
        "SHA-256(\"abc\") matches FIPS test vector",
    );
}

fn test_sha256_hex_format(ctx: &mut TestContext) {
    let hex = sha256_hex(b"abc");
    let expected = *b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    ctx.expect_eq(hex, expected, "sha256_hex produces correct lowercase hex");
}

fn test_password_roundtrip(ctx: &mut TestContext) {
    let encoded = hash_password("s3cret");
    ctx.expect_true(encoded.starts_with("pbkdf2-sha256$"), "password hash uses PBKDF2-SHA256 format");
    ctx.expect_true(verify_password("s3cret", &encoded), "password hash verifies with original password");
    ctx.expect_true(!verify_password("wrong", &encoded), "password hash rejects wrong password");
}

fn test_password_needs_rehash(ctx: &mut TestContext) {
    ctx.expect_true(
        needs_rehash("5f4dcc3b5aa765d61d8327deb882cf99"),
        "legacy MD5 password records require rehash",
    );
}
