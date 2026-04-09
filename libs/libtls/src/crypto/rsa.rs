//! RSA signature verification (PKCS#1 v1.5 and PSS).
//!
//! Only verification is implemented (TLS client doesn't need signing).
//! Supports RSA-2048 and RSA-4096 key sizes.

use alloc::vec::Vec;
use crate::crypto::bignum::BigUint;
use crate::crypto::sha256;
use crate::crypto::sha384;

/// RSA public key (modulus n, exponent e).
#[derive(Clone, Debug)]
pub struct RsaPublicKey {
    pub n: BigUint,
    pub e: BigUint,
}

/// Verify an RSA PKCS#1 v1.5 signature.
///
/// 1. Compute s^e mod n
/// 2. Check PKCS#1 v1.5 padding: 0x00 0x01 [0xFF...] 0x00 [DigestInfo]
/// 3. Compare hash in DigestInfo with provided `hash`
pub fn verify_pkcs1_v15(
    key: &RsaPublicKey,
    signature: &[u8],
    hash: &[u8],
    hash_algorithm: HashAlgorithm,
) -> bool {
    let key_len = (key.n.bit_len() + 7) / 8;

    // signature must be exactly key_len bytes
    if signature.len() != key_len {
        return false;
    }

    // s^e mod n
    let s = BigUint::from_be_bytes(signature);
    let m = s.powmod(&key.e, &key.n);
    let em = m.to_be_bytes_padded(key_len);

    // Check PKCS#1 v1.5 structure: 0x00 0x01 [0xFF padding] 0x00 [DigestInfo]
    if em.len() < 11 || em[0] != 0x00 || em[1] != 0x01 {
        return false;
    }

    // Find end of 0xFF padding
    let mut i = 2;
    while i < em.len() && em[i] == 0xFF {
        i += 1;
    }
    if i < 10 || i >= em.len() || em[i] != 0x00 {
        return false;
    }
    i += 1; // skip the 0x00 separator

    let digest_info = &em[i..];

    // Build expected DigestInfo: DER-encoded AlgorithmIdentifier + hash
    let expected = build_digest_info(hash, hash_algorithm);

    // Constant-time comparison
    if digest_info.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for j in 0..expected.len() {
        diff |= digest_info[j] ^ expected[j];
    }
    diff == 0
}

/// Verify an RSA-PSS signature (RFC 8017 Section 8.1.2).
///
/// Used for `rsa_pss_rsae_sha256` in TLS 1.3.
pub fn verify_pss(
    key: &RsaPublicKey,
    signature: &[u8],
    message_hash: &[u8],
    hash_algorithm: HashAlgorithm,
) -> bool {
    let em_bits = key.n.bit_len() - 1;
    let em_len = (em_bits + 7) / 8;
    let h_len = hash_algorithm.digest_len();
    let s_len = h_len; // salt length = hash length (standard for TLS)

    if signature.len() != (key.n.bit_len() + 7) / 8 {
        return false;
    }

    // s^e mod n
    let s = BigUint::from_be_bytes(signature);
    let m = s.powmod(&key.e, &key.n);
    let em = m.to_be_bytes_padded(em_len);

    // Check rightmost byte is 0xBC
    if em.is_empty() || em[em.len() - 1] != 0xBC {
        return false;
    }

    if em_len < h_len + s_len + 2 {
        return false;
    }

    let db_len = em_len - h_len - 1;
    let masked_db = &em[..db_len];
    let h = &em[db_len..db_len + h_len];

    // Check top bits are zero
    let top_mask = 0xFFu8 << (8 - (8 * em_len - em_bits));
    if masked_db[0] & top_mask != 0 {
        return false;
    }

    // dbMask = MGF1(H, dbLen)
    let db_mask = mgf1(h, db_len, hash_algorithm);

    // DB = maskedDB XOR dbMask
    let mut db = Vec::with_capacity(db_len);
    for i in 0..db_len {
        db.push(masked_db[i] ^ db_mask[i]);
    }

    // Clear top bits
    db[0] &= !top_mask;

    // Check DB format: 0x00...0x00 0x01 salt
    let ps_len = db_len - s_len - 1;
    for i in 0..ps_len {
        if db[i] != 0x00 {
            return false;
        }
    }
    if db[ps_len] != 0x01 {
        return false;
    }
    let salt = &db[ps_len + 1..];

    // M' = 0x00 00 00 00 00 00 00 00 || mHash || salt
    let mut m_prime = Vec::with_capacity(8 + h_len + s_len);
    m_prime.extend_from_slice(&[0u8; 8]);
    m_prime.extend_from_slice(message_hash);
    m_prime.extend_from_slice(salt);

    // H' = Hash(M')
    let h_prime = hash_algorithm.hash(&m_prime);

    // Compare H and H'
    let mut diff = 0u8;
    for i in 0..h_len {
        diff |= h[i] ^ h_prime[i];
    }
    diff == 0
}

/// MGF1 mask generation function (RFC 8017 Appendix B.2.1).
fn mgf1(seed: &[u8], mask_len: usize, hash_alg: HashAlgorithm) -> Vec<u8> {
    let h_len = hash_alg.digest_len();
    let iterations = (mask_len + h_len - 1) / h_len;
    let mut output = Vec::with_capacity(iterations * h_len);

    for counter in 0..iterations as u32 {
        let mut input = Vec::with_capacity(seed.len() + 4);
        input.extend_from_slice(seed);
        input.extend_from_slice(&counter.to_be_bytes());
        let digest = hash_alg.hash(&input);
        output.extend_from_slice(&digest);
    }

    output.truncate(mask_len);
    output
}

/// Hash algorithms used with RSA signatures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha256,
    Sha384,
}

impl HashAlgorithm {
    pub fn digest_len(&self) -> usize {
        match self {
            HashAlgorithm::Sha256 => 32,
            HashAlgorithm::Sha384 => 48,
        }
    }

    pub fn hash(&self, data: &[u8]) -> Vec<u8> {
        match self {
            HashAlgorithm::Sha256 => sha256::sha256(data).to_vec(),
            HashAlgorithm::Sha384 => sha384::sha384(data).to_vec(),
        }
    }
}

/// Build PKCS#1 v1.5 DigestInfo (DER encoded).
fn build_digest_info(hash: &[u8], alg: HashAlgorithm) -> Vec<u8> {
    // DigestInfo ::= SEQUENCE {
    //   digestAlgorithm AlgorithmIdentifier,
    //   digest OCTET STRING
    // }
    let (oid, hash_len) = match alg {
        HashAlgorithm::Sha256 => (
            // OID 2.16.840.1.101.3.4.2.1 (SHA-256)
            &[0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01,
              0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00][..],
            32usize,
        ),
        HashAlgorithm::Sha384 => (
            // OID 2.16.840.1.101.3.4.2.2 (SHA-384)
            &[0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01,
              0x65, 0x03, 0x04, 0x02, 0x02, 0x05, 0x00][..],
            48usize,
        ),
    };

    let inner_len = oid.len() + 2 + hash_len; // AlgId + OCTET STRING tag+len + hash
    let mut di = Vec::with_capacity(2 + inner_len);
    di.push(0x30); // SEQUENCE
    di.push(inner_len as u8);
    di.extend_from_slice(oid);
    di.push(0x04); // OCTET STRING
    di.push(hash_len as u8);
    di.extend_from_slice(hash);
    di
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_digest_info_sha256() {
        let hash = [0u8; 32];
        let di = build_digest_info(&hash, HashAlgorithm::Sha256);
        // Should start with SEQUENCE tag
        assert_eq!(di[0], 0x30);
        // Total DigestInfo for SHA-256: 19 bytes prefix + 32 bytes hash = 51 bytes
        assert_eq!(di.len(), 51);
    }

    #[test]
    fn test_rsa_small_verify() {
        // Small RSA: n=3233, e=17, d=2753
        // Sign: hash=42, sig = 42^2753 mod 3233
        let n = BigUint::from_u64(3233);
        let e = BigUint::from_u64(17);
        let d = BigUint::from_u64(2753);

        let msg = BigUint::from_u64(42);
        let sig = msg.powmod(&d, &n);
        let recovered = sig.powmod(&e, &n);
        assert_eq!(recovered, msg);
    }

    #[test]
    fn test_mgf1() {
        // RFC 8017 test (simplified verification)
        let seed = sha256::sha256(b"test");
        let mask = mgf1(&seed, 48, HashAlgorithm::Sha256);
        assert_eq!(mask.len(), 48);
        // Mask should be deterministic
        let mask2 = mgf1(&seed, 48, HashAlgorithm::Sha256);
        assert_eq!(mask, mask2);
    }
}
