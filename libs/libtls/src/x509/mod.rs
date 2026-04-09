//! X.509 certificate parsing for TLS.
//!
//! Currently uses a trust-all model (same as the BearSSL wrapper it replaces).
//! Extracts the server's public key from the certificate for signature
//! verification during the TLS handshake.

pub mod asn1;

use alloc::vec::Vec;
use crate::crypto::bignum::BigUint;
use crate::crypto::rsa::RsaPublicKey;
use crate::crypto::p256::P256Point;
use asn1::*;

/// The type of public key found in a certificate.
#[derive(Debug)]
pub enum PublicKey {
    Rsa(RsaPublicKey),
    EcdsaP256(P256Point),
}

/// Signature algorithm used by the certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureAlgorithm {
    Sha256WithRsa,
    Sha384WithRsa,
    Sha256WithEcdsa,
    Sha384WithEcdsa,
    RsaPssSha256,
    Unknown,
}

/// Parsed X.509 certificate (minimal fields for TLS).
#[derive(Debug)]
pub struct Certificate {
    pub public_key: PublicKey,
    pub signature_algorithm: SignatureAlgorithm,
}

/// Parse an X.509 certificate from DER encoding.
/// Extracts the public key and signature algorithm.
///
/// Does NOT verify the certificate chain (trust-all model).
pub fn parse_certificate(der: &[u8]) -> Option<Certificate> {
    // Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signatureValue }
    let cert_seq = parse_tlv(der)?;
    if cert_seq.tag != TAG_SEQUENCE {
        return None;
    }

    let mut iter = TlvIterator::new(cert_seq.value);

    // TBSCertificate ::= SEQUENCE { ... }
    let tbs = iter.next()?;
    if tbs.tag != TAG_SEQUENCE {
        return None;
    }

    // Parse TBSCertificate to extract public key
    let public_key = parse_tbs_certificate(tbs.value)?;

    // SignatureAlgorithm
    let sig_alg_tlv = iter.next()?;
    let signature_algorithm = parse_algorithm_identifier(sig_alg_tlv.value);

    Some(Certificate { public_key, signature_algorithm })
}

/// Parse TBSCertificate to extract the SubjectPublicKeyInfo.
fn parse_tbs_certificate(data: &[u8]) -> Option<PublicKey> {
    let mut iter = TlvIterator::new(data);

    // version [0] EXPLICIT INTEGER (optional)
    let first = iter.next()?;
    let _version_or_serial = if first.tag == TAG_CONTEXT_0 {
        // Version present, next is serial
        iter.next()?;
        first
    } else {
        // No version, first is serial
        first
    };

    // serialNumber INTEGER — skip
    // signature AlgorithmIdentifier — skip
    let _signature = iter.next()?;
    // issuer Name — skip
    let _issuer = iter.next()?;
    // validity Validity — skip
    let _validity = iter.next()?;
    // subject Name — skip
    let _subject = iter.next()?;

    // subjectPublicKeyInfo SubjectPublicKeyInfo
    let spki = iter.next()?;
    if spki.tag != TAG_SEQUENCE {
        return None;
    }

    parse_subject_public_key_info(spki.value)
}

/// Parse SubjectPublicKeyInfo to extract the public key.
fn parse_subject_public_key_info(data: &[u8]) -> Option<PublicKey> {
    let mut iter = TlvIterator::new(data);

    // AlgorithmIdentifier ::= SEQUENCE { algorithm OID, parameters ANY }
    let alg_id = iter.next()?;
    if alg_id.tag != TAG_SEQUENCE {
        return None;
    }
    let alg_oid = get_first_oid(alg_id.value)?;

    // subjectPublicKey BIT STRING
    let pub_key_bits = iter.next()?;
    if pub_key_bits.tag != TAG_BIT_STRING {
        return None;
    }
    // BIT STRING: first byte is number of unused bits (usually 0)
    if pub_key_bits.value.is_empty() {
        return None;
    }
    let key_data = &pub_key_bits.value[1..]; // skip unused-bits byte

    // rsaEncryption: 1.2.840.113549.1.1.1
    if oid_matches(&alg_oid, &[1, 2, 840, 113549, 1, 1, 1]) {
        return parse_rsa_public_key(key_data);
    }

    // ecPublicKey: 1.2.840.10045.2.1
    if oid_matches(&alg_oid, &[1, 2, 840, 10045, 2, 1]) {
        return parse_ec_public_key(key_data);
    }

    None
}

/// Parse RSA public key from DER (PKCS#1 RSAPublicKey).
fn parse_rsa_public_key(data: &[u8]) -> Option<PublicKey> {
    // RSAPublicKey ::= SEQUENCE { modulus INTEGER, publicExponent INTEGER }
    let seq = parse_tlv(data)?;
    if seq.tag != TAG_SEQUENCE {
        return None;
    }

    let mut iter = TlvIterator::new(seq.value);
    let n_tlv = iter.next()?;
    let e_tlv = iter.next()?;

    if n_tlv.tag != TAG_INTEGER || e_tlv.tag != TAG_INTEGER {
        return None;
    }

    let n = BigUint::from_be_bytes(parse_der_integer(n_tlv.value));
    let e = BigUint::from_be_bytes(parse_der_integer(e_tlv.value));

    Some(PublicKey::Rsa(RsaPublicKey { n, e }))
}

/// Parse EC public key (uncompressed point).
fn parse_ec_public_key(data: &[u8]) -> Option<PublicKey> {
    // For P-256: 0x04 || x (32 bytes) || y (32 bytes) = 65 bytes
    let point = P256Point::from_uncompressed(data)?;
    Some(PublicKey::EcdsaP256(point))
}

/// Extract the first OID from an AlgorithmIdentifier SEQUENCE.
fn get_first_oid(data: &[u8]) -> Option<Vec<u32>> {
    let tlv = parse_tlv(data)?;
    if tlv.tag != TAG_OID {
        return None;
    }
    Some(parse_oid(tlv.value))
}

/// Check if a parsed OID matches an expected OID.
fn oid_matches(parsed: &[u32], expected: &[u32]) -> bool {
    parsed == expected
}

/// Parse AlgorithmIdentifier to determine the signature algorithm.
fn parse_algorithm_identifier(data: &[u8]) -> SignatureAlgorithm {
    let mut iter = TlvIterator::new(data);
    let oid_tlv = match iter.next() {
        Some(t) if t.tag == TAG_OID => t,
        _ => return SignatureAlgorithm::Unknown,
    };
    let oid = parse_oid(oid_tlv.value);

    // sha256WithRSAEncryption: 1.2.840.113549.1.1.11
    if oid_matches(&oid, &[1, 2, 840, 113549, 1, 1, 11]) {
        return SignatureAlgorithm::Sha256WithRsa;
    }
    // sha384WithRSAEncryption: 1.2.840.113549.1.1.12
    if oid_matches(&oid, &[1, 2, 840, 113549, 1, 1, 12]) {
        return SignatureAlgorithm::Sha384WithRsa;
    }
    // id-RSASSA-PSS: 1.2.840.113549.1.1.10
    if oid_matches(&oid, &[1, 2, 840, 113549, 1, 1, 10]) {
        return SignatureAlgorithm::RsaPssSha256; // Assume SHA-256 for now
    }
    // ecdsa-with-SHA256: 1.2.840.10045.4.3.2
    if oid_matches(&oid, &[1, 2, 840, 10045, 4, 3, 2]) {
        return SignatureAlgorithm::Sha256WithEcdsa;
    }
    // ecdsa-with-SHA384: 1.2.840.10045.4.3.3
    if oid_matches(&oid, &[1, 2, 840, 10045, 4, 3, 3]) {
        return SignatureAlgorithm::Sha384WithEcdsa;
    }

    SignatureAlgorithm::Unknown
}
