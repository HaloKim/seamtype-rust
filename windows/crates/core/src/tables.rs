//! Read-only views over the tables build.rs bakes into the binary (see build.rs for the layout).

#![allow(dead_code)]

/// Sorted words, each with an optional byte (the SCOWL level for English).
pub struct WordTable {
    bytes: &'static [u8],
    count: usize,
    with_values: bool,
    offsets_at: usize,
    values_at: usize,
    words_at: usize,
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn f32_at(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

impl WordTable {
    pub fn new(bytes: &'static [u8]) -> WordTable {
        let count = u32_at(bytes, 0) as usize;
        let with_values = bytes[4] != 0;
        let offsets_at = 5;
        let values_at = offsets_at + (count + 1) * 4;
        let words_at = values_at + if with_values { count } else { 0 };
        WordTable { bytes, count, with_values, offsets_at, values_at, words_at }
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn word(&self, i: usize) -> &'static [u8] {
        let start = u32_at(self.bytes, self.offsets_at + i * 4) as usize;
        let end = u32_at(self.bytes, self.offsets_at + (i + 1) * 4) as usize;
        &self.bytes[self.words_at + start..self.words_at + end]
    }

    fn index(&self, word: &str) -> Option<usize> {
        let key = word.as_bytes();
        let (mut lo, mut hi) = (0, self.count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.word(mid).cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid),
            }
        }
        None
    }

    pub fn contains(&self, word: &str) -> bool {
        self.index(word).is_some()
    }

    /// The word's byte, or None when it isn't listed.
    pub fn value(&self, word: &str) -> Option<u8> {
        let i = self.index(word)?;
        Some(if self.with_values { self.bytes[self.values_at + i] } else { 0 })
    }

    /// Every word, in table order.
    pub fn words(&self) -> impl Iterator<Item = &'static str> + '_ {
        (0..self.count).map(|i| std::str::from_utf8(self.word(i)).unwrap_or(""))
    }
}

/// Little-endian f32s.
pub struct FloatArray {
    bytes: &'static [u8],
    at: usize,
    len: usize,
}

impl FloatArray {
    pub fn new(bytes: &'static [u8], at: usize, len: usize) -> FloatArray {
        assert!(at + len * 4 <= bytes.len());
        FloatArray { bytes, at, len }
    }

    pub fn get(&self, i: usize) -> f32 {
        debug_assert!(i < self.len);
        f32_at(self.bytes, self.at + i * 4)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Sorted little-endian u32 keys with an f32 each.
pub struct SparseFloats {
    bytes: &'static [u8],
    keys_at: usize,
    values_at: usize,
    len: usize,
}

impl SparseFloats {
    pub fn new(bytes: &'static [u8], keys_at: usize, len: usize) -> SparseFloats {
        let values_at = keys_at + len * 4;
        assert!(values_at + len * 4 <= bytes.len());
        SparseFloats { bytes, keys_at, values_at, len }
    }

    pub fn get(&self, key: u32) -> Option<f32> {
        let (mut lo, mut hi) = (0, self.len);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let k = u32_at(self.bytes, self.keys_at + mid * 4);
            match k.cmp(&key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(f32_at(self.bytes, self.values_at + mid * 4)),
            }
        }
        None
    }
}

pub(crate) fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32_at(bytes, at)
}

pub(crate) fn read_f32(bytes: &[u8], at: usize) -> f32 {
    f32_at(bytes, at)
}
