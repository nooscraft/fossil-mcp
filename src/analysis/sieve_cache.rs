//! SIEVE cache — a simple, efficient eviction cache.
//!
//! Based on the SIEVE algorithm from the NSDI'24 paper:
//! *"SIEVE is Simpler than LRU: an Efficient Turn-Key Eviction Algorithm for
//! Web Caches"*.
//!
//! SIEVE uses a single "hand" pointer that sweeps through a circular buffer of
//! entries.  On a cache hit the entry's `visited` bit is set.  When an eviction
//! is needed the hand advances, clearing `visited` bits as it goes, until it
//! finds an entry with `visited == false` — that entry is evicted.

use std::collections::HashMap;
use std::hash::Hash;

/// Internal cache entry storing the value and its visited bit.
struct CacheEntry<V> {
    value: V,
    visited: bool,
}

/// A fixed-capacity cache that uses the SIEVE eviction algorithm.
///
/// # Type parameters
///
/// - `K` — key type (must be `Hash + Eq + Clone`).
/// - `V` — value type.
pub struct SieveCache<K, V> {
    capacity: usize,
    /// Maps a key to its slot index inside `entries`.
    index: HashMap<K, usize>,
    /// Circular buffer of entries.  `None` marks a free slot.
    entries: Vec<Option<(K, CacheEntry<V>)>>,
    /// Current hand position (index into `entries`).
    hand: usize,
    /// Number of occupied entries.
    size: usize,
    /// Lifetime hit counter.
    hits: u64,
    /// Lifetime miss counter.
    misses: u64,
}

impl<K: Hash + Eq + Clone, V> SieveCache<K, V> {
    /// Creates a new `SieveCache` with the given maximum capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "SieveCache capacity must be > 0");

        let mut entries = Vec::with_capacity(capacity);
        entries.resize_with(capacity, || None);

        Self {
            capacity,
            index: HashMap::with_capacity(capacity),
            entries,
            hand: 0,
            size: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Returns a shared reference to the value associated with `key`, or `None`
    /// if the key is not present.
    ///
    /// On a hit the entry's visited bit is set to `true`.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        if let Some(&slot) = self.index.get(key) {
            if let Some((_, ref mut entry)) = self.entries[slot] {
                entry.visited = true;
                self.hits += 1;
                return Some(&entry.value);
            }
        }
        self.misses += 1;
        None
    }

    /// Returns a mutable reference to the value associated with `key`, or
    /// `None` if the key is not present.
    ///
    /// On a hit the entry's visited bit is set to `true`.
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        if let Some(&slot) = self.index.get(key) {
            if let Some((_, ref mut entry)) = self.entries[slot] {
                entry.visited = true;
                self.hits += 1;
                return Some(&mut entry.value);
            }
        }
        self.misses += 1;
        None
    }

    /// Inserts a key-value pair into the cache.
    ///
    /// If the key already exists, its value is updated and the visited bit is
    /// set.  If the cache is full, the SIEVE eviction algorithm is used to
    /// remove an entry before inserting the new one.
    pub fn insert(&mut self, key: K, value: V) {
        // Update existing entry.
        if let Some(&slot) = self.index.get(&key) {
            if let Some((_, ref mut entry)) = self.entries[slot] {
                entry.value = value;
                entry.visited = true;
                return;
            }
        }

        // Evict if at capacity.
        if self.size >= self.capacity {
            self.evict();
        }

        // Find a free slot.  After eviction there is guaranteed to be one.
        let free_slot = self.find_free_slot();
        self.entries[free_slot] = Some((
            key.clone(),
            CacheEntry {
                value,
                visited: false,
            },
        ));
        self.index.insert(key, free_slot);
        self.size += 1;
    }

    /// Removes the entry for `key` and returns its value, or `None` if not
    /// present.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        if let Some(slot) = self.index.remove(key) {
            if let Some((_, entry)) = self.entries[slot].take() {
                self.size -= 1;
                return Some(entry.value);
            }
        }
        None
    }

    /// Returns `true` if the cache contains the given key.
    pub fn contains_key(&self, key: &K) -> bool {
        self.index.contains_key(key)
    }

    /// Returns the number of entries currently in the cache.
    pub fn len(&self) -> usize {
        self.size
    }

    /// Returns `true` if the cache contains no entries.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Returns the maximum capacity of the cache.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the total number of cache hits since creation (or last clear).
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Returns the total number of cache misses since creation (or last clear).
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Returns the hit ratio as a value in `[0.0, 1.0]`.
    ///
    /// Returns `0.0` if there have been no accesses.
    pub fn hit_ratio(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 0.0;
        }
        self.hits as f64 / total as f64
    }

    /// Removes all entries and resets hit/miss counters.
    pub fn clear(&mut self) {
        self.index.clear();
        for slot in &mut self.entries {
            *slot = None;
        }
        self.hand = 0;
        self.size = 0;
        self.hits = 0;
        self.misses = 0;
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// SIEVE eviction: advance the hand, clearing visited bits, until we find
    /// an entry with `visited == false` — then evict it.
    fn evict(&mut self) {
        loop {
            if let Some((ref key, ref mut entry)) = self.entries[self.hand] {
                if entry.visited {
                    entry.visited = false;
                    self.advance_hand();
                } else {
                    // Evict this entry.
                    let evicted_key = key.clone();
                    self.entries[self.hand] = None;
                    self.index.remove(&evicted_key);
                    self.size -= 1;
                    // Leave hand pointing at the now-free slot.
                    return;
                }
            } else {
                // Empty slot — should not happen when cache is full, but
                // advance defensively.
                self.advance_hand();
            }
        }
    }

    /// Advance the hand pointer circularly.
    fn advance_hand(&mut self) {
        self.hand = (self.hand + 1) % self.capacity;
    }

    /// Linear scan for a free (`None`) slot, starting from the current hand
    /// position.
    fn find_free_slot(&self) -> usize {
        let mut idx = self.hand;
        for _ in 0..self.capacity {
            if self.entries[idx].is_none() {
                return idx;
            }
            idx = (idx + 1) % self.capacity;
        }
        unreachable!("find_free_slot called but no free slot exists");
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_insert_and_get() {
        let mut cache = SieveCache::new(4);
        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("c", 3);

        assert_eq!(cache.get(&"a"), Some(&1));
        assert_eq!(cache.get(&"b"), Some(&2));
        assert_eq!(cache.get(&"c"), Some(&3));
        assert_eq!(cache.get(&"d"), None);
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn test_update_existing_key() {
        let mut cache = SieveCache::new(4);
        cache.insert("a", 1);
        cache.insert("a", 42);

        assert_eq!(cache.get(&"a"), Some(&42));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_eviction_behavior() {
        let mut cache = SieveCache::new(3);

        // Fill to capacity.
        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("c", 3);
        assert_eq!(cache.len(), 3);

        // Access "a" and "b" to set their visited bits.
        assert_eq!(cache.get(&"a"), Some(&1));
        assert_eq!(cache.get(&"b"), Some(&2));

        // Insert "d" — must evict one entry.
        // "c" was never accessed (visited=false), so it should be evicted.
        cache.insert("d", 4);
        assert_eq!(cache.len(), 3);

        // "c" should be gone.
        assert!(!cache.contains_key(&"c"));
        // The others survive.
        assert!(cache.contains_key(&"a"));
        assert!(cache.contains_key(&"b"));
        assert!(cache.contains_key(&"d"));
    }

    #[test]
    fn test_eviction_clears_visited_bits() {
        let mut cache = SieveCache::new(3);

        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("c", 3);

        // Access all three so every entry has visited=true.
        cache.get(&"a");
        cache.get(&"b");
        cache.get(&"c");

        // Now insert "d".  The hand must clear visited bits before evicting.
        cache.insert("d", 4);
        assert_eq!(cache.len(), 3);

        // One of {a, b, c} was evicted (the first one the hand reached after
        // clearing all visited bits).
        let remaining: Vec<bool> = ["a", "b", "c"]
            .iter()
            .map(|k| cache.contains_key(k))
            .collect();
        let evicted_count = remaining.iter().filter(|&&v| !v).count();
        assert_eq!(
            evicted_count, 1,
            "Exactly one of the original entries should have been evicted"
        );
        assert!(cache.contains_key(&"d"));
    }

    #[test]
    fn test_hit_ratio_tracking() {
        let mut cache = SieveCache::new(4);
        cache.insert("a", 1);

        // 1 hit
        cache.get(&"a");
        // 2 misses
        cache.get(&"x");
        cache.get(&"y");

        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 2);

        let ratio = cache.hit_ratio();
        let expected = 1.0 / 3.0;
        assert!(
            (ratio - expected).abs() < 1e-9,
            "Expected hit ratio ~{expected}, got {ratio}"
        );
    }

    #[test]
    fn test_capacity_and_clear() {
        let mut cache = SieveCache::new(2);
        assert_eq!(cache.capacity(), 2);

        cache.insert(1, "one");
        cache.insert(2, "two");
        assert_eq!(cache.len(), 2);
        assert!(!cache.is_empty());

        cache.clear();
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
        assert_eq!(cache.hits(), 0);
        assert_eq!(cache.misses(), 0);
        assert_eq!(cache.get(&1), None);
    }

    #[test]
    fn test_remove() {
        let mut cache = SieveCache::new(4);
        cache.insert("a", 10);
        cache.insert("b", 20);

        assert_eq!(cache.remove(&"a"), Some(10));
        assert_eq!(cache.len(), 1);
        assert!(!cache.contains_key(&"a"));

        // Removing a non-existent key returns None.
        assert_eq!(cache.remove(&"z"), None);
    }

    #[test]
    fn test_get_mut() {
        let mut cache = SieveCache::new(4);
        cache.insert("a", vec![1, 2, 3]);

        if let Some(v) = cache.get_mut(&"a") {
            v.push(4);
        }

        assert_eq!(cache.get(&"a"), Some(&vec![1, 2, 3, 4]));
    }

    #[test]
    fn test_single_capacity() {
        let mut cache = SieveCache::new(1);
        cache.insert("a", 1);
        assert_eq!(cache.get(&"a"), Some(&1));

        // Inserting another key must evict the only entry.
        cache.insert("b", 2);
        assert_eq!(cache.len(), 1);
        assert!(!cache.contains_key(&"a"));
        assert_eq!(cache.get(&"b"), Some(&2));
    }

    #[test]
    fn test_hit_ratio_no_accesses() {
        let cache: SieveCache<String, i32> = SieveCache::new(4);
        assert!((cache.hit_ratio() - 0.0).abs() < f64::EPSILON);
    }
}
