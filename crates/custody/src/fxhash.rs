//! A fast non-cryptographic hasher for per-operation maps keyed by the
//! operation's own identifiers. Maps using it are never iterated where their
//! order could reach output.
use std::hash::{BuildHasherDefault, Hasher};

#[derive(Clone, Copy, Default)]
pub(crate) struct FxHasher {
    hash: u64,
}

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.add(u64::from_le_bytes(*word));
        }
        if !rest.is_empty() {
            let mut word = [0; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word));
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub(crate) type Build = BuildHasherDefault<FxHasher>;
pub(crate) type HashMap<K, V> = std::collections::HashMap<K, V, Build>;
pub(crate) type HashSet<K> = std::collections::HashSet<K, Build>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fx_maps_find_every_key() {
        let keys: Vec<String> = (0..2000).map(|n| format!("{n:064x}")).collect();
        let mut map = HashMap::default();
        for (at, key) in keys.iter().enumerate() {
            map.insert(key.as_str(), at);
        }
        for (at, key) in keys.iter().enumerate() {
            assert_eq!(map[key.as_str()], at);
        }
        assert!(!map.contains_key("missing"));
        let mut a = FxHasher::default();
        let mut b = FxHasher::default();
        a.write(b"abcdefgh1");
        b.write(b"abcdefgh2");
        assert_ne!(a.finish(), b.finish());
    }
}
