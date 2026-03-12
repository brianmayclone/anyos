//! HashSet — a hash set built on top of HashMap.

use super::hash_map::HashMap;
use alloc::vec::Vec;
use core::hash::Hash;

/// A hash set using FNV-1a hashing with open addressing.
///
/// Wrapper around `HashMap<T, ()>` for set semantics.
#[derive(Clone)]
pub struct HashSet<T> {
    map: HashMap<T, ()>,
}

impl<T: Hash + Eq> HashSet<T> {
    /// Create an empty HashSet.
    pub fn new() -> Self {
        HashSet { map: HashMap::new() }
    }

    /// Create a HashSet with pre-allocated capacity.
    pub fn with_capacity(cap: usize) -> Self {
        HashSet { map: HashMap::with_capacity(cap) }
    }

    /// Number of elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the set is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Insert a value. Returns `true` if the value was not already present.
    pub fn insert(&mut self, value: T) -> bool {
        self.map.insert(value, ()).is_none()
    }

    /// Check if a value exists.
    pub fn contains(&self, value: &T) -> bool {
        self.map.contains_key(value)
    }

    /// Remove a value. Returns `true` if the value was present.
    pub fn remove(&mut self, value: &T) -> bool {
        self.map.remove(value).is_some()
    }

    /// Clear all elements.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Iterate over values.
    pub fn iter(&self) -> Iter<'_, T> {
        Iter { inner: self.map.keys() }
    }

    /// Returns the intersection of `self` and `other` as a new set.
    pub fn intersection<'a>(&'a self, other: &'a HashSet<T>) -> Vec<&'a T> {
        self.iter().filter(|v| other.contains(v)).collect()
    }

    /// Returns the union of `self` and `other` as a new set (owned).
    pub fn union_with(&self, other: &HashSet<T>) -> HashSet<T>
    where
        T: Clone,
    {
        let mut result = self.clone();
        for v in other.iter() {
            result.insert(v.clone());
        }
        result
    }

    /// Returns the difference `self - other` as references.
    pub fn difference<'a>(&'a self, other: &'a HashSet<T>) -> Vec<&'a T> {
        self.iter().filter(|v| !other.contains(v)).collect()
    }
}

impl<T: Hash + Eq> Default for HashSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Hash + Eq + core::fmt::Debug> core::fmt::Debug for HashSet<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

// ── Iterator ────────────────────────────────────────────────────────────

pub struct Iter<'a, T> {
    inner: super::hash_map::Keys<'a, T, ()>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

// ── IntoIterator ────────────────────────────────────────────────────────

pub struct IntoIter<T> {
    inner: super::hash_map::IntoIter<T, ()>,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(k, _)| k)
    }
}

impl<T: Hash + Eq> IntoIterator for HashSet<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter { inner: self.map.into_iter() }
    }
}

impl<'a, T: Hash + Eq> IntoIterator for &'a HashSet<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// ── FromIterator ────────────────────────────────────────────────────────

impl<T: Hash + Eq> core::iter::FromIterator<T> for HashSet<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut set = HashSet::new();
        for v in iter {
            set.insert(v);
        }
        set
    }
}
