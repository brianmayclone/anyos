//! Multi-precision unsigned integer arithmetic for RSA and P-256.
//!
//! Uses a variable-length array of u64 limbs (little-endian: limb 0 is least
//! significant). Provides modular exponentiation for RSA signature verification
//! and basic arithmetic for P-256 field operations.

use alloc::vec::Vec;

/// Big unsigned integer stored as little-endian u64 limbs.
#[derive(Clone, Debug)]
pub struct BigUint {
    pub limbs: Vec<u64>,
}

impl BigUint {
    pub fn zero() -> Self {
        Self {
            limbs: alloc::vec![0],
        }
    }

    pub fn one() -> Self {
        Self {
            limbs: alloc::vec![1],
        }
    }

    pub fn from_u64(v: u64) -> Self {
        Self {
            limbs: alloc::vec![v],
        }
    }

    /// Decode from big-endian bytes (as used in DER/X.509).
    pub fn from_be_bytes(data: &[u8]) -> Self {
        if data.is_empty() {
            return Self::zero();
        }
        // Skip leading zeros
        let start = data.iter().position(|&b| b != 0).unwrap_or(data.len());
        let data = &data[start..];
        if data.is_empty() {
            return Self::zero();
        }

        let num_limbs = (data.len() + 7) / 8;
        let mut limbs = Vec::with_capacity(num_limbs);

        // Process 8 bytes at a time from the end (big-endian → little-endian limbs)
        let mut i = data.len();
        while i > 0 {
            let start = if i >= 8 { i - 8 } else { 0 };
            let chunk = &data[start..i];
            let mut val = 0u64;
            for &b in chunk {
                val = (val << 8) | (b as u64);
            }
            limbs.push(val);
            i = start;
        }

        let mut r = Self { limbs };
        r.trim();
        r
    }

    /// Encode to big-endian bytes.
    pub fn to_be_bytes(&self) -> Vec<u8> {
        if self.is_zero() {
            return alloc::vec![0];
        }
        let mut result = Vec::new();
        let mut started = false;
        for &limb in self.limbs.iter().rev() {
            let bytes = limb.to_be_bytes();
            for &b in &bytes {
                if !started && b == 0 {
                    continue;
                }
                started = true;
                result.push(b);
            }
        }
        if result.is_empty() {
            result.push(0);
        }
        result
    }

    /// Encode to big-endian bytes with fixed width (zero-padded).
    pub fn to_be_bytes_padded(&self, len: usize) -> Vec<u8> {
        let raw = self.to_be_bytes();
        if raw.len() >= len {
            return raw[raw.len() - len..].to_vec();
        }
        let mut out = alloc::vec![0u8; len];
        out[len - raw.len()..].copy_from_slice(&raw);
        out
    }

    pub fn is_zero(&self) -> bool {
        self.limbs.iter().all(|&l| l == 0)
    }

    pub fn bit_len(&self) -> usize {
        let n = self.limbs.len();
        if n == 0 {
            return 0;
        }
        let top = self.limbs[n - 1];
        if top == 0 {
            return 0;
        }
        (n - 1) * 64 + (64 - top.leading_zeros() as usize)
    }

    /// Get bit at position `i` (0 = LSB).
    pub fn bit(&self, i: usize) -> u64 {
        let limb_idx = i / 64;
        let bit_idx = i % 64;
        if limb_idx >= self.limbs.len() {
            0
        } else {
            (self.limbs[limb_idx] >> bit_idx) & 1
        }
    }

    fn trim(&mut self) {
        while self.limbs.len() > 1 && *self.limbs.last().unwrap() == 0 {
            self.limbs.pop();
        }
    }

    /// Compare: -1 if self < other, 0 if equal, 1 if self > other.
    pub fn cmp(&self, other: &Self) -> i32 {
        let a = &self.limbs;
        let b = &other.limbs;
        let al = a.len();
        let bl = b.len();
        let max = al.max(bl);
        for i in (0..max).rev() {
            let av = if i < al { a[i] } else { 0 };
            let bv = if i < bl { b[i] } else { 0 };
            if av < bv {
                return -1;
            }
            if av > bv {
                return 1;
            }
        }
        0
    }

    /// self + other
    pub fn add(&self, other: &Self) -> Self {
        let max_len = self.limbs.len().max(other.limbs.len());
        let mut result = Vec::with_capacity(max_len + 1);
        let mut carry = 0u64;
        for i in 0..max_len {
            let a = if i < self.limbs.len() {
                self.limbs[i]
            } else {
                0
            };
            let b = if i < other.limbs.len() {
                other.limbs[i]
            } else {
                0
            };
            let (sum1, c1) = a.overflowing_add(b);
            let (sum2, c2) = sum1.overflowing_add(carry);
            result.push(sum2);
            carry = (c1 as u64) + (c2 as u64);
        }
        if carry > 0 {
            result.push(carry);
        }
        let mut r = Self { limbs: result };
        r.trim();
        r
    }

    /// self - other (panics if other > self).
    pub fn sub(&self, other: &Self) -> Self {
        let mut result = Vec::with_capacity(self.limbs.len());
        let mut borrow = 0u64;
        for i in 0..self.limbs.len() {
            let a = self.limbs[i];
            let b = if i < other.limbs.len() {
                other.limbs[i]
            } else {
                0
            };
            let (diff1, c1) = a.overflowing_sub(b);
            let (diff2, c2) = diff1.overflowing_sub(borrow);
            result.push(diff2);
            borrow = (c1 as u64) + (c2 as u64);
        }
        let mut r = Self { limbs: result };
        r.trim();
        r
    }

    /// self * other
    pub fn mul(&self, other: &Self) -> Self {
        let n = self.limbs.len() + other.limbs.len();
        let mut result = alloc::vec![0u64; n];
        for i in 0..self.limbs.len() {
            let mut carry = 0u128;
            for j in 0..other.limbs.len() {
                let prod = (self.limbs[i] as u128) * (other.limbs[j] as u128)
                    + (result[i + j] as u128)
                    + carry;
                result[i + j] = prod as u64;
                carry = prod >> 64;
            }
            result[i + other.limbs.len()] += carry as u64;
        }
        let mut r = Self { limbs: result };
        r.trim();
        r
    }

    /// self % modulus (using repeated subtraction for small cases, or
    /// schoolbook division for general cases).
    pub fn rem(&self, modulus: &Self) -> Self {
        if modulus.is_zero() {
            return Self::zero();
        }
        if self.cmp(modulus) < 0 {
            return self.clone();
        }
        // Schoolbook long division
        let mut remainder = Self::zero();
        let bits = self.bit_len();
        for i in (0..bits).rev() {
            // Shift remainder left by 1
            remainder = remainder.shl1();
            // Set bit 0 from dividend
            if self.bit(i) != 0 {
                remainder.limbs[0] |= 1;
            }
            // Subtract modulus if remainder >= modulus
            if remainder.cmp(modulus) >= 0 {
                remainder = remainder.sub(modulus);
            }
        }
        remainder
    }

    /// Shift left by 1 bit.
    fn shl1(&self) -> Self {
        let mut result = Vec::with_capacity(self.limbs.len() + 1);
        let mut carry = 0u64;
        for &limb in &self.limbs {
            result.push((limb << 1) | carry);
            carry = limb >> 63;
        }
        if carry > 0 {
            result.push(carry);
        }
        Self { limbs: result }
    }

    /// Modular multiplication: (self * other) % modulus
    pub fn mulmod(&self, other: &Self, modulus: &Self) -> Self {
        self.mul(other).rem(modulus)
    }

    /// Modular exponentiation: self^exp % modulus
    /// Uses left-to-right binary method (square-and-multiply).
    pub fn powmod(&self, exp: &Self, modulus: &Self) -> Self {
        if modulus.is_zero() {
            return Self::zero();
        }
        let mut result = Self::one();
        let mut base = self.rem(modulus);
        let bits = exp.bit_len();

        for i in 0..bits {
            if exp.bit(i) != 0 {
                result = result.mulmod(&base, modulus);
            }
            base = base.mulmod(&base, modulus);
        }
        result
    }
}

impl PartialEq for BigUint {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == 0
    }
}

impl Eq for BigUint {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_be_bytes() {
        let n = BigUint::from_be_bytes(&[0x01, 0x00]);
        assert_eq!(n.limbs[0], 256);
    }

    #[test]
    fn test_to_be_bytes_roundtrip() {
        let data = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x01];
        let n = BigUint::from_be_bytes(&data);
        assert_eq!(n.to_be_bytes(), data.to_vec());
    }

    #[test]
    fn test_mul() {
        let a = BigUint::from_u64(12345);
        let b = BigUint::from_u64(67890);
        let c = a.mul(&b);
        assert_eq!(c.limbs[0], 12345u64 * 67890);
    }

    #[test]
    fn test_powmod() {
        // 3^10 mod 7 = 59049 mod 7 = 4
        let base = BigUint::from_u64(3);
        let exp = BigUint::from_u64(10);
        let modulus = BigUint::from_u64(7);
        let result = base.powmod(&exp, &modulus);
        assert_eq!(result.limbs[0], 4);
    }

    #[test]
    fn test_powmod_rsa_small() {
        // Simulate small RSA: m = c^d mod n
        // n = 3233 (= 61*53), e = 17, d = 2753
        // encrypt: 65^17 mod 3233 = 2790
        // decrypt: 2790^2753 mod 3233 = 65
        let m = BigUint::from_u64(65);
        let e = BigUint::from_u64(17);
        let n = BigUint::from_u64(3233);
        let c = m.powmod(&e, &n);
        assert_eq!(c.limbs[0], 2790);

        let d = BigUint::from_u64(2753);
        let decrypted = c.powmod(&d, &n);
        assert_eq!(decrypted.limbs[0], 65);
    }
}
