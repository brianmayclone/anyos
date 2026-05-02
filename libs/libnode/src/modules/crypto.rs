use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use libjs::value::{JsObject, JsValue};
use libjs::vm::{native_fn, Vm};

use super::buffer::{buffer_from_bytes, buffer_to_bytes, is_buffer_like};
use super::util::object;

const HASH_ALG_KEY: &str = "__node_crypto_hash_alg__";
const HASH_DATA_KEY: &str = "__node_crypto_hash_data__";

pub fn module() -> JsValue {
    let mut module = JsObject::new();
    module.set(
        String::from("createHash"),
        native_fn("createHash", create_hash),
    );
    module.set(
        String::from("getHashes"),
        native_fn("getHashes", get_hashes),
    );
    module.set(
        String::from("randomBytes"),
        native_fn("randomBytes", random_bytes),
    );
    module.set(
        String::from("randomFillSync"),
        native_fn("randomFillSync", random_fill_sync),
    );
    module.set(
        String::from("randomUUID"),
        native_fn("randomUUID", random_uuid),
    );
    module.set(
        String::from("timingSafeEqual"),
        native_fn("timingSafeEqual", timing_safe_equal),
    );
    object(module)
}

fn create_hash(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let alg = normalize_alg(
        args.first()
            .map(|value| value.to_js_string())
            .unwrap_or_default(),
    );
    let mut hash = JsObject::new();
    hash.set_hidden(String::from(HASH_ALG_KEY), JsValue::String(alg));
    hash.set_hidden(String::from(HASH_DATA_KEY), JsValue::new_array(Vec::new()));
    hash.set(String::from("update"), native_fn("update", hash_update));
    hash.set(String::from("digest"), native_fn("digest", hash_digest));
    object(hash)
}

fn hash_update(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let mut data = bytes_from(args.first().unwrap_or(&JsValue::Undefined));
    let mut current = bytes_from(&vm.current_this.get_property(HASH_DATA_KEY));
    current.append(&mut data);
    vm.current_this
        .set_property(String::from(HASH_DATA_KEY), bytes_array(current));
    vm.current_this.clone()
}

fn hash_digest(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let alg = vm.current_this.get_property(HASH_ALG_KEY).to_js_string();
    let data = bytes_from(&vm.current_this.get_property(HASH_DATA_KEY));
    let digest = digest_bytes(&alg, &data);
    match args
        .first()
        .map(|value| normalize_encoding(&value.to_js_string()))
        .as_deref()
    {
        Some("hex") => JsValue::String(hex_encode(&digest)),
        Some("base64") => JsValue::String(base64_encode(&digest)),
        Some("latin1") | Some("binary") => {
            JsValue::String(digest.into_iter().map(|byte| byte as char).collect())
        }
        _ => buffer_from_bytes(digest),
    }
}

fn get_hashes(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::new_array(
        ["md5", "sha1", "sha256", "sha384", "sha512"]
            .iter()
            .map(|name| JsValue::String(String::from(*name)))
            .collect(),
    )
}

fn random_bytes(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let len = args
        .first()
        .map(|value| value.to_number().max(0.0) as usize)
        .unwrap_or(0);
    let mut bytes = vec![0; len];
    anyos_std::sys::random(&mut bytes);
    let out = buffer_from_bytes(bytes);
    if let Some(callback) = args.get(1) {
        if matches!(callback, JsValue::Function(_)) {
            vm.call_value(callback, &[JsValue::Null, out.clone()], JsValue::Undefined);
        }
    }
    out
}

fn random_fill_sync(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let Some(target) = args.first() else {
        return JsValue::Undefined;
    };
    let mut bytes = buffer_to_bytes(target);
    anyos_std::sys::random(&mut bytes);
    if is_buffer_like(target) {
        // Buffer payload is hidden/internal in this VM; return a fresh compatible Buffer.
        return buffer_from_bytes(bytes);
    }
    JsValue::new_array(
        bytes
            .into_iter()
            .map(|byte| JsValue::Number(byte as f64))
            .collect(),
    )
}

fn random_uuid(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let mut bytes = [0u8; 16];
    anyos_std::sys::random(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    JsValue::String(format_uuid(&bytes))
}

fn timing_safe_equal(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let left = bytes_from(args.first().unwrap_or(&JsValue::Undefined));
    let right = bytes_from(args.get(1).unwrap_or(&JsValue::Undefined));
    if left.len() != right.len() {
        return JsValue::Bool(false);
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    JsValue::Bool(diff == 0)
}

fn digest_bytes(alg: &str, data: &[u8]) -> Vec<u8> {
    match normalize_alg(alg) {
        alg if alg == "md5" => md5_digest(data).to_vec(),
        alg if alg == "sha1" => libtls::crypto::sha1::sha1(data).to_vec(),
        alg if alg == "sha256" => libtls::crypto::sha256::sha256(data).to_vec(),
        alg if alg == "sha384" => libtls::crypto::sha384::sha384(data).to_vec(),
        alg if alg == "sha512" => libtls::crypto::sha512::sha512(data).to_vec(),
        _ => libtls::crypto::sha256::sha256(data).to_vec(),
    }
}

fn bytes_from(value: &JsValue) -> Vec<u8> {
    match value {
        JsValue::String(text) => text.as_bytes().to_vec(),
        JsValue::Array(array) => array
            .borrow()
            .to_dense_vec()
            .into_iter()
            .map(|value| value.to_number().clamp(0.0, 255.0) as u8)
            .collect(),
        value if is_buffer_like(value) => buffer_to_bytes(value),
        value => value.to_js_string().into_bytes(),
    }
}

fn bytes_array(bytes: Vec<u8>) -> JsValue {
    JsValue::new_array(
        bytes
            .into_iter()
            .map(|byte| JsValue::Number(byte as f64))
            .collect(),
    )
}

fn normalize_alg(alg: impl AsRef<str>) -> String {
    alg.as_ref()
        .to_ascii_lowercase()
        .replace('-', "")
        .replace('_', "")
}

fn normalize_encoding(encoding: &str) -> String {
    encoding.to_ascii_lowercase().replace('-', "")
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = bytes.get(i + 1).copied().unwrap_or(0);
        let b2 = bytes.get(i + 2).copied().unwrap_or(0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < bytes.len() {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

fn format_uuid(bytes: &[u8; 16]) -> String {
    let hex = hex_encode(bytes);
    alloc::format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn md5_digest(input: &[u8]) -> [u8; 16] {
    let s: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];

    let orig_len_bits = (input.len() as u64) * 8;
    let pad_len = {
        let rem = input.len() % 64;
        if rem < 56 {
            56 - rem
        } else {
            120 - rem
        }
    };
    let total = input.len() + pad_len + 8;
    let mut msg = vec![0u8; total];
    msg[..input.len()].copy_from_slice(input);
    msg[input.len()] = 0x80;
    msg[total - 8..total].copy_from_slice(&orig_len_bits.to_le_bytes());

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, word) in m.iter_mut().enumerate() {
            let base = i * 4;
            *word = u32::from_le_bytes([
                chunk[base],
                chunk[base + 1],
                chunk[base + 2],
                chunk[base + 3],
            ]);
        }

        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            let sum = a.wrapping_add(f).wrapping_add(k[i]).wrapping_add(m[g]);
            b = b.wrapping_add(sum.rotate_left(s[i]));
            a = temp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut result = [0u8; 16];
    result[0..4].copy_from_slice(&a0.to_le_bytes());
    result[4..8].copy_from_slice(&b0.to_le_bytes());
    result[8..12].copy_from_slice(&c0.to_le_bytes());
    result[12..16].copy_from_slice(&d0.to_le_bytes());
    result
}
