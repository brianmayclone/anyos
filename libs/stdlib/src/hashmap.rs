//! Minimal HashMap for no_std environments.
//!
//! Uses FNV-1a hashing with open-addressing (linear probing).
//! Power-of-2 table size, resizes at 75% load factor.
//!
//! Supports all standard operations: insert, get, get_mut, remove,
//! contains_key, len, iter, iter_mut, keys, values, clear.

use alloc::vec;
use alloc::vec::Vec;
use core::hash::{Hash, Hasher};

// ── FNV-1a Hasher ────────────────────────────────────────────────────────

/// FNV-1a 64-bit hasher — fast, no_std compatible, good distribution.
pub struct FnvHasher(u64);

impl FnvHasher {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x00000100000001B3;

    pub fn new() -> Self {
        FnvHasher(Self::OFFSET)
    }
}

impl Hasher for FnvHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }
}

/// Compute FNV-1a hash for any `Hash` type.
#[inline]
fn hash_key<K: Hash>(key: &K) -> u64 {
    let mut hasher = FnvHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

// ── HashMap ──────────────────────────────────────────────────────────────

const INITIAL_CAPACITY: usize = 16;
const LOAD_FACTOR_NUM: usize = 3;
const LOAD_FACTOR_DEN: usize = 4; // resize at 75% load

/// A hash map using FNV-1a hashing with open addressing (linear probing).
pub struct HashMap<K, V> {
    buckets: Vec<Option<(K, V)>>,
    len: usize,
}

impl<K: Hash + Eq, V> HashMap<K, V> {
    /// Create an empty HashMap.
    pub fn new() -> Self {
        HashMap {
            buckets: Vec::new(),
            len: 0,
        }
    }

    /// Create a HashMap with pre-allocated capacity.
    pub fn with_capacity(cap: usize) -> Self {
        let cap = cap.next_power_of_two().max(INITIAL_CAPACITY);
        let mut buckets = Vec::with_capacity(cap);
        for _ in 0..cap {
            buckets.push(None);
        }
        HashMap { buckets, len: 0 }
    }

    /// Number of entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the map is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of allocated bucket slots.
    pub fn capacity(&self) -> usize {
        self.buckets.len()
    }

    /// Insert a key-value pair. Returns the previous value if the key existed.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        if self.buckets.is_empty() {
            self.resize(INITIAL_CAPACITY);
        } else if self.len * LOAD_FACTOR_DEN >= self.buckets.len() * LOAD_FACTOR_NUM {
            self.resize(self.buckets.len() * 2);
        }

        let mask = self.buckets.len() - 1;
        let mut idx = (hash_key(&key) as usize) & mask;

        loop {
            match &self.buckets[idx] {
                Some((k, _)) if *k == key => {
                    // Key exists — replace value
                    let old = self.buckets[idx].take().unwrap();
                    self.buckets[idx] = Some((key, value));
                    return Some(old.1);
                }
                Some(_) => {
                    // Occupied by different key — linear probe
                    idx = (idx + 1) & mask;
                }
                None => {
                    // Empty slot — insert
                    self.buckets[idx] = Some((key, value));
                    self.len += 1;
                    return None;
                }
            }
        }
    }

    /// Get a reference to the value for a key.
    pub fn get(&self, key: &K) -> Option<&V> {
        if self.buckets.is_empty() {
            return None;
        }
        let mask = self.buckets.len() - 1;
        let mut idx = (hash_key(key) as usize) & mask;

        for _ in 0..self.buckets.len() {
            match &self.buckets[idx] {
                Some((k, v)) if *k == *key => return Some(v),
                Some(_) => idx = (idx + 1) & mask,
                None => return None,
            }
        }
        None
    }

    /// Get a mutable reference to the value for a key.
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        if self.buckets.is_empty() {
            return None;
        }
        let mask = self.buckets.len() - 1;
        let mut idx = (hash_key(key) as usize) & mask;

        for _ in 0..self.buckets.len() {
            match &self.buckets[idx] {
                Some((k, _)) if *k == *key => {
                    return self.buckets[idx].as_mut().map(|(_, v)| v);
                }
                Some(_) => idx = (idx + 1) & mask,
                None => return None,
            }
        }
        None
    }

    /// Check if a key exists.
    pub fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    /// Remove a key-value pair, returning the value if it existed.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        if self.buckets.is_empty() {
            return None;
        }
        let mask = self.buckets.len() - 1;
        let mut idx = (hash_key(key) as usize) & mask;

        for _ in 0..self.buckets.len() {
            match &self.buckets[idx] {
                Some((k, _)) if *k == *key => {
                    let removed = self.buckets[idx].take().unwrap();
                    self.len -= 1;
                    // Backward-shift deletion to maintain probe chain integrity
                    self.backward_shift(idx);
                    return Some(removed.1);
                }
                Some(_) => idx = (idx + 1) & mask,
                None => return None,
            }
        }
        None
    }

    /// Clear all entries (keeps allocation).
    pub fn clear(&mut self) {
        for bucket in &mut self.buckets {
            *bucket = None;
        }
        self.len = 0;
    }

    /// Iterate over key-value pairs.
    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter { inner: self.buckets.iter() }
    }

    /// Iterate over key-value pairs (mutable values).
    pub fn iter_mut(&mut self) -> IterMut<'_, K, V> {
        IterMut { inner: self.buckets.iter_mut() }
    }

    /// Iterate over keys.
    pub fn keys(&self) -> Keys<'_, K, V> {
        Keys { inner: self.iter() }
    }

    /// Iterate over values.
    pub fn values(&self) -> Values<'_, K, V> {
        Values { inner: self.iter() }
    }

    // ── Internal ─────────────────────────────────────────────────────

    /// Resize the bucket array to a new capacity.
    fn resize(&mut self, new_cap: usize) {
        let new_cap = new_cap.next_power_of_two();
        let mut new_buckets: Vec<Option<(K, V)>> = Vec::with_capacity(new_cap);
        for _ in 0..new_cap {
            new_buckets.push(None);
        }
        let mask = new_cap - 1;

        for bucket in self.buckets.drain(..) {
            if let Some((k, v)) = bucket {
                let mut idx = (hash_key(&k) as usize) & mask;
                loop {
                    if new_buckets[idx].is_none() {
                        new_buckets[idx] = Some((k, v));
                        break;
                    }
                    idx = (idx + 1) & mask;
                }
            }
        }

        self.buckets = new_buckets;
    }

    /// Backward-shift deletion: after removing an entry, shift subsequent
    /// entries backward to fill the gap and maintain probe chains.
    fn backward_shift(&mut self, removed_idx: usize) {
        let mask = self.buckets.len() - 1;
        let mut empty = removed_idx;
        let mut probe = (removed_idx + 1) & mask;

        loop {
            if self.buckets[probe].is_none() {
                break;
            }
            // Check if this entry's ideal position is at or before the empty slot
            let ideal = {
                let (k, _) = self.buckets[probe].as_ref().unwrap();
                (hash_key(k) as usize) & mask
            };
            // If the ideal position is "between" empty and probe (modular), shift it
            if (probe > empty && (ideal <= empty || ideal > probe))
                || (probe < empty && ideal <= empty && ideal > probe)
            {
                self.buckets.swap(empty, probe);
                empty = probe;
            }
            probe = (probe + 1) & mask;
        }
    }
}

impl<K: Hash + Eq, V> Default for HashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Hash + Eq + core::fmt::Debug, V: core::fmt::Debug> core::fmt::Debug for HashMap<K, V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

// ── Iterators ────────────────────────────────────────────────────────────

pub struct Iter<'a, K, V> {
    inner: core::slice::Iter<'a, Option<(K, V)>>,
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.inner.next() {
                Some(Some((k, v))) => return Some((k, v)),
                Some(None) => continue,
                None => return None,
            }
        }
    }
}

pub struct IterMut<'a, K, V> {
    inner: core::slice::IterMut<'a, Option<(K, V)>>,
}

impl<'a, K, V> Iterator for IterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.inner.next() {
                Some(Some((k, v))) => return Some((k, v)),
                Some(None) => continue,
                None => return None,
            }
        }
    }
}

pub struct Keys<'a, K, V> {
    inner: Iter<'a, K, V>,
}

impl<'a, K, V> Iterator for Keys<'a, K, V> {
    type Item = &'a K;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(k, _)| k)
    }
}

pub struct Values<'a, K, V> {
    inner: Iter<'a, K, V>,
}

impl<'a, K, V> Iterator for Values<'a, K, V> {
    type Item = &'a V;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(_, v)| v)
    }
}

// ── IntoIterator ─────────────────────────────────────────────────────────

pub struct IntoIter<K, V> {
    inner: alloc::vec::IntoIter<Option<(K, V)>>,
}

impl<K, V> Iterator for IntoIter<K, V> {
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.inner.next() {
                Some(Some(pair)) => return Some(pair),
                Some(None) => continue,
                None => return None,
            }
        }
    }
}

impl<K, V> IntoIterator for HashMap<K, V> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter { inner: self.buckets.into_iter() }
    }
}

impl<'a, K: Hash + Eq, V> IntoIterator for &'a HashMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// ── FromIterator ─────────────────────────────────────────────────────────

impl<K: Hash + Eq, V> core::iter::FromIterator<(K, V)> for HashMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let (lower, _) = iter.size_hint();
        let mut map = HashMap::with_capacity(lower.max(INITIAL_CAPACITY));
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

// ── Index ────────────────────────────────────────────────────────────────

impl<K: Hash + Eq, V> core::ops::Index<&K> for HashMap<K, V> {
    type Output = V;

    fn index(&self, key: &K) -> &V {
        self.get(key).expect("key not found in HashMap")
    }
}
