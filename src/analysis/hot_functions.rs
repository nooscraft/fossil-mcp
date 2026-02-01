//! Hot-function tracking using the HeavyKeeper probabilistic data structure.
//!
//! During graph construction every call-edge target is recorded via
//! [`HotFunctionTracker::record_call`].  After the pass completes,
//! [`HotFunctionTracker::top_functions`] returns the approximate top-K most
//! referenced functions as `(name_hash, count)` pairs.
//!
//! The underlying [`sketch_oxide::HeavyKeeper`] provides space-efficient,
//! streaming top-K tracking with bounded error.

use sketch_oxide::HeavyKeeper;
use xxhash_rust::xxh3::xxh3_64;

/// Tracks the most frequently called functions using a probabilistic
/// heavy-hitter sketch.
pub struct HotFunctionTracker {
    keeper: HeavyKeeper,
}

impl HotFunctionTracker {
    /// Creates a new tracker that maintains the top `top_k` heaviest callee
    /// names.
    ///
    /// # Panics
    ///
    /// Panics if `HeavyKeeper` initialization fails (should not happen with
    /// the hardcoded error parameters).
    pub fn new(top_k: usize) -> Self {
        let keeper = HeavyKeeper::new(top_k, 0.001, 0.01)
            .expect("HeavyKeeper initialization should not fail");
        Self { keeper }
    }

    /// Records a single call to the function identified by `callee_name`.
    ///
    /// The name is hashed internally and fed into the HeavyKeeper sketch.
    pub fn record_call(&mut self, callee_name: &str) {
        self.keeper.update(callee_name.as_bytes());
    }

    /// Returns the current top-K functions as `(name_hash, count)` pairs,
    /// ordered by descending count.
    pub fn top_functions(&self) -> Vec<(u64, u32)> {
        self.keeper.top_k()
    }

    /// Convenience: returns the number of items currently tracked in the
    /// top-K list.
    pub fn tracked_count(&self) -> usize {
        self.keeper.top_k().len()
    }

    /// Hash a function name with xxh3 — useful for correlating the
    /// `name_hash` values returned by `top_functions` with actual names.
    pub fn hash_name(name: &str) -> u64 {
        xxh3_64(name.as_bytes())
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_tracker_is_empty() {
        let tracker = HotFunctionTracker::new(10);
        assert!(
            tracker.top_functions().is_empty(),
            "Fresh tracker should have no top functions"
        );
    }

    #[test]
    fn test_record_single_function() {
        let mut tracker = HotFunctionTracker::new(10);
        for _ in 0..100 {
            tracker.record_call("process_data");
        }

        let top = tracker.top_functions();
        assert!(
            !top.is_empty(),
            "Top functions should contain at least one entry after recording"
        );

        // The dominant function should appear with a count >= 50
        // (HeavyKeeper may under-count slightly).
        let max_count = top.iter().map(|&(_, c)| c).max().unwrap_or(0);
        assert!(
            max_count >= 50,
            "Expected dominant function count >= 50, got {max_count}"
        );
    }

    #[test]
    fn test_multiple_functions_ranking() {
        let mut tracker = HotFunctionTracker::new(10);

        // "hot" called 500 times, "warm" 50 times, "cold" 5 times.
        for _ in 0..500 {
            tracker.record_call("hot");
        }
        for _ in 0..50 {
            tracker.record_call("warm");
        }
        for _ in 0..5 {
            tracker.record_call("cold");
        }

        let top = tracker.top_functions();
        assert!(
            top.len() >= 2,
            "Should track at least the two most frequent functions"
        );

        // The highest count entry should correspond to "hot".
        let highest = top.iter().max_by_key(|&&(_, c)| c).unwrap();
        assert!(
            highest.1 >= 100,
            "Hottest function count should be >= 100, got {}",
            highest.1
        );
    }

    #[test]
    fn test_hash_name_deterministic() {
        let h1 = HotFunctionTracker::hash_name("my_func");
        let h2 = HotFunctionTracker::hash_name("my_func");
        assert_eq!(h1, h2, "hash_name should be deterministic");

        let h3 = HotFunctionTracker::hash_name("other_func");
        assert_ne!(
            h1, h3,
            "Different names should (very likely) hash differently"
        );
    }

    #[test]
    fn test_tracked_count() {
        let mut tracker = HotFunctionTracker::new(5);
        tracker.record_call("alpha");
        tracker.record_call("beta");
        tracker.record_call("gamma");

        let count = tracker.tracked_count();
        assert!(
            count <= 5,
            "Tracked count should not exceed top_k, got {count}"
        );
    }
}
