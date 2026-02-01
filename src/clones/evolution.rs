//! Clone evolution tracking across codebase snapshots.
//!
//! Compares function fingerprints between two snapshots to detect how clone
//! relationships change over time: new clones appearing, existing clones diverging,
//! independent functions converging, or functions being added/removed entirely.
//!
//! The primary consumer is maintenance tooling that needs to flag diverging clones
//! (functions that were once similar but are drifting apart), since those represent
//! the highest risk of inconsistent bug fixes.

use std::collections::{HashMap, HashSet};

use super::fingerprint_store::{FingerprintStore, FunctionFingerprint, SnapshotId};
use super::minhash::MinHashDetector;

/// The type of evolution event detected between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvolutionEventType {
    /// A new function appeared in the later snapshot (not present in the earlier one).
    Appeared,
    /// A function from the earlier snapshot is absent in the later one.
    Disappeared,
    /// A function pair that was a clone (similarity >= threshold) has dropped below
    /// the threshold -- they are diverging and may need synchronized maintenance.
    Diverged,
    /// A function pair that was NOT a clone has risen above the threshold --
    /// independently-written code is converging (possible copy-paste in progress).
    Converged,
    /// A function pair that was a clone and remains a clone (above threshold in both
    /// snapshots). Similarity may have changed slightly but the pair is still coupled.
    Unchanged,
}

/// A single clone evolution event between two snapshots.
#[derive(Debug, Clone)]
pub struct CloneEvolutionEvent {
    /// The pair of functions involved, identified as `(file::name, file::name)`.
    pub clone_pair: (String, String),
    /// What happened to this pair between the two snapshots.
    pub event_type: EvolutionEventType,
    /// Earlier snapshot ID.
    pub snapshot_from: SnapshotId,
    /// Later snapshot ID.
    pub snapshot_to: SnapshotId,
    /// Jaccard similarity of the pair in the earlier snapshot (0.0 if not present).
    pub similarity_before: f64,
    /// Jaccard similarity of the pair in the later snapshot (0.0 if not present).
    pub similarity_after: f64,
}

/// Tracks how clone relationships evolve between codebase snapshots.
///
/// Given a [`FingerprintStore`] containing multiple snapshots, this tracker
/// compares function pairs across two snapshots and emits [`CloneEvolutionEvent`]s
/// describing what changed.
pub struct CloneEvolutionTracker {
    /// The underlying fingerprint store with snapshot data.
    pub store: FingerprintStore,
    /// Jaccard similarity threshold for considering a pair a "clone".
    similarity_threshold: f64,
}

impl CloneEvolutionTracker {
    /// Creates a new tracker with an empty store and the given similarity threshold.
    ///
    /// # Arguments
    ///
    /// * `similarity_threshold` - Jaccard similarity at or above which a function
    ///   pair is considered a clone. Typical values: 0.5-0.8.
    pub fn new(similarity_threshold: f64) -> Self {
        Self {
            store: FingerprintStore::new(),
            similarity_threshold,
        }
    }

    /// Compares two snapshots and returns all evolution events.
    ///
    /// # Algorithm
    ///
    /// 1. Build a lookup of functions by `(file, name)` for both snapshots.
    /// 2. Identify functions that appeared or disappeared.
    /// 3. For every pair of functions present in BOTH snapshots, compute Jaccard
    ///    similarity (via `MinHashDetector::exact_jaccard` on the minhash sketches)
    ///    in both the "before" and "after" snapshots.
    /// 4. Classify each pair as Diverged, Converged, or Unchanged based on whether
    ///    the pair crosses the similarity threshold.
    ///
    /// Returns an empty `Vec` if either snapshot ID is not found.
    pub fn track_evolution(&self, snapshot_a: &str, snapshot_b: &str) -> Vec<CloneEvolutionEvent> {
        let fps_a = match self.store.get_snapshot(snapshot_a) {
            Some(fps) => fps,
            None => return Vec::new(),
        };
        let fps_b = match self.store.get_snapshot(snapshot_b) {
            Some(fps) => fps,
            None => return Vec::new(),
        };

        // Index fingerprints by (file, name) for O(1) lookup.
        let index_a = build_function_index(fps_a);
        let index_b = build_function_index(fps_b);

        let keys_a: HashSet<&(String, String)> = index_a.keys().collect();
        let keys_b: HashSet<&(String, String)> = index_b.keys().collect();

        let mut events = Vec::new();

        // Functions that appeared (in B but not in A).
        for key in keys_b.difference(&keys_a) {
            events.push(CloneEvolutionEvent {
                clone_pair: (format!("{}::{}", key.0, key.1), String::new()),
                event_type: EvolutionEventType::Appeared,
                snapshot_from: snapshot_a.to_string(),
                snapshot_to: snapshot_b.to_string(),
                similarity_before: 0.0,
                similarity_after: 0.0,
            });
        }

        // Functions that disappeared (in A but not in B).
        for key in keys_a.difference(&keys_b) {
            events.push(CloneEvolutionEvent {
                clone_pair: (format!("{}::{}", key.0, key.1), String::new()),
                event_type: EvolutionEventType::Disappeared,
                snapshot_from: snapshot_a.to_string(),
                snapshot_to: snapshot_b.to_string(),
                similarity_before: 0.0,
                similarity_after: 0.0,
            });
        }

        // Functions present in both snapshots -- compare all pairs.
        let common_keys: Vec<&(String, String)> = keys_a.intersection(&keys_b).copied().collect();

        // For each unique pair of common functions, compare their evolution.
        for i in 0..common_keys.len() {
            for j in (i + 1)..common_keys.len() {
                let key_i = common_keys[i];
                let key_j = common_keys[j];

                let fp_i_a = &index_a[key_i];
                let fp_j_a = &index_a[key_j];
                let fp_i_b = &index_b[key_i];
                let fp_j_b = &index_b[key_j];

                let sim_before =
                    MinHashDetector::exact_jaccard(&fp_i_a.minhash_sketch, &fp_j_a.minhash_sketch);
                let sim_after =
                    MinHashDetector::exact_jaccard(&fp_i_b.minhash_sketch, &fp_j_b.minhash_sketch);

                let was_clone = sim_before >= self.similarity_threshold;
                let is_clone = sim_after >= self.similarity_threshold;

                let event_type = match (was_clone, is_clone) {
                    (true, false) => EvolutionEventType::Diverged,
                    (false, true) => EvolutionEventType::Converged,
                    (true, true) => EvolutionEventType::Unchanged,
                    // Both below threshold in both snapshots -- not interesting.
                    (false, false) => continue,
                };

                events.push(CloneEvolutionEvent {
                    clone_pair: (
                        format!("{}::{}", key_i.0, key_i.1),
                        format!("{}::{}", key_j.0, key_j.1),
                    ),
                    event_type,
                    snapshot_from: snapshot_a.to_string(),
                    snapshot_to: snapshot_b.to_string(),
                    similarity_before: sim_before,
                    similarity_after: sim_after,
                });
            }
        }

        events
    }

    /// Returns only the Diverged events between two snapshots.
    ///
    /// Diverging clones are the highest-priority maintenance risk: code that was
    /// once duplicated and similar is drifting apart, meaning bug fixes applied to
    /// one copy may be missing from the other.
    pub fn detect_diverging_clones(
        &self,
        snapshot_a: &str,
        snapshot_b: &str,
    ) -> Vec<CloneEvolutionEvent> {
        self.track_evolution(snapshot_a, snapshot_b)
            .into_iter()
            .filter(|e| e.event_type == EvolutionEventType::Diverged)
            .collect()
    }
}

/// Builds a lookup from `(file, name)` to the corresponding fingerprint.
///
/// If multiple fingerprints share the same `(file, name)`, the last one wins.
fn build_function_index(
    fingerprints: &[FunctionFingerprint],
) -> HashMap<(String, String), &FunctionFingerprint> {
    let mut index = HashMap::new();
    for fp in fingerprints {
        index.insert((fp.file.clone(), fp.name.clone()), fp);
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clones::fingerprint_store::FunctionFingerprint;

    /// Helper: creates a fingerprint with the given minhash sketch.
    fn make_fp(file: &str, name: &str, minhash: Vec<u64>) -> FunctionFingerprint {
        FunctionFingerprint {
            file: file.to_string(),
            name: name.to_string(),
            start_line: 1,
            end_line: 10,
            minhash_sketch: minhash,
            simhash: 0,
            content_hash: 0,
            timestamp: "t0".to_string(),
        }
    }

    #[test]
    fn test_function_appeared() {
        let mut tracker = CloneEvolutionTracker::new(0.5);

        // Snapshot A has one function; snapshot B adds a second.
        tracker
            .store
            .add_snapshot("a".to_string(), vec![make_fp("f.py", "foo", vec![1, 2, 3])]);
        tracker.store.add_snapshot(
            "b".to_string(),
            vec![
                make_fp("f.py", "foo", vec![1, 2, 3]),
                make_fp("f.py", "bar", vec![10, 20, 30]),
            ],
        );

        let events = tracker.track_evolution("a", "b");
        let appeared: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == EvolutionEventType::Appeared)
            .collect();

        assert_eq!(appeared.len(), 1);
        assert!(appeared[0].clone_pair.0.contains("bar"));
    }

    #[test]
    fn test_function_disappeared() {
        let mut tracker = CloneEvolutionTracker::new(0.5);

        tracker.store.add_snapshot(
            "a".to_string(),
            vec![
                make_fp("f.py", "foo", vec![1, 2, 3]),
                make_fp("f.py", "bar", vec![10, 20, 30]),
            ],
        );
        tracker
            .store
            .add_snapshot("b".to_string(), vec![make_fp("f.py", "foo", vec![1, 2, 3])]);

        let events = tracker.track_evolution("a", "b");
        let disappeared: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == EvolutionEventType::Disappeared)
            .collect();

        assert_eq!(disappeared.len(), 1);
        assert!(disappeared[0].clone_pair.0.contains("bar"));
    }

    #[test]
    fn test_clone_pair_diverged() {
        let mut tracker = CloneEvolutionTracker::new(0.5);

        // In snapshot A, foo and bar share most shingles (high Jaccard).
        // In snapshot B, bar has completely different shingles (low Jaccard).
        let shared: Vec<u64> = (0..100).collect();
        let diverged: Vec<u64> = (500..600).collect();

        tracker.store.add_snapshot(
            "a".to_string(),
            vec![
                make_fp("f.py", "foo", shared.clone()),
                make_fp("f.py", "bar", shared.clone()),
            ],
        );
        tracker.store.add_snapshot(
            "b".to_string(),
            vec![
                make_fp("f.py", "foo", shared),
                make_fp("f.py", "bar", diverged),
            ],
        );

        let events = tracker.track_evolution("a", "b");
        let diverged_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == EvolutionEventType::Diverged)
            .collect();

        assert_eq!(diverged_events.len(), 1);
        assert!(
            diverged_events[0].similarity_before >= 0.5,
            "Before similarity should be >= threshold, got {}",
            diverged_events[0].similarity_before
        );
        assert!(
            diverged_events[0].similarity_after < 0.5,
            "After similarity should be < threshold, got {}",
            diverged_events[0].similarity_after
        );
    }

    #[test]
    fn test_clone_pair_converged() {
        let mut tracker = CloneEvolutionTracker::new(0.5);

        // In snapshot A, foo and bar are completely different.
        // In snapshot B, bar has become similar to foo.
        let set_a: Vec<u64> = (0..100).collect();
        let set_b_different: Vec<u64> = (500..600).collect();

        tracker.store.add_snapshot(
            "a".to_string(),
            vec![
                make_fp("f.py", "foo", set_a.clone()),
                make_fp("f.py", "bar", set_b_different),
            ],
        );
        tracker.store.add_snapshot(
            "b".to_string(),
            vec![
                make_fp("f.py", "foo", set_a.clone()),
                make_fp("f.py", "bar", set_a),
            ],
        );

        let events = tracker.track_evolution("a", "b");
        let converged: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == EvolutionEventType::Converged)
            .collect();

        assert_eq!(converged.len(), 1);
        assert!(
            converged[0].similarity_before < 0.5,
            "Before similarity should be < threshold, got {}",
            converged[0].similarity_before
        );
        assert!(
            converged[0].similarity_after >= 0.5,
            "After similarity should be >= threshold, got {}",
            converged[0].similarity_after
        );
    }

    #[test]
    fn test_clone_pair_unchanged() {
        let mut tracker = CloneEvolutionTracker::new(0.5);

        // In both snapshots, foo and bar are identical clones.
        let shared: Vec<u64> = (0..100).collect();

        tracker.store.add_snapshot(
            "a".to_string(),
            vec![
                make_fp("f.py", "foo", shared.clone()),
                make_fp("f.py", "bar", shared.clone()),
            ],
        );
        tracker.store.add_snapshot(
            "b".to_string(),
            vec![
                make_fp("f.py", "foo", shared.clone()),
                make_fp("f.py", "bar", shared),
            ],
        );

        let events = tracker.track_evolution("a", "b");
        let unchanged: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == EvolutionEventType::Unchanged)
            .collect();

        assert_eq!(unchanged.len(), 1);
        assert!(
            (unchanged[0].similarity_before - 1.0).abs() < f64::EPSILON,
            "Identical clones should have similarity 1.0 before"
        );
    }

    #[test]
    fn test_detect_diverging_clones_filters_correctly() {
        let mut tracker = CloneEvolutionTracker::new(0.5);

        let shared: Vec<u64> = (0..100).collect();
        let diverged: Vec<u64> = (500..600).collect();
        let unrelated_a: Vec<u64> = (1000..1100).collect();
        let unrelated_b: Vec<u64> = (2000..2100).collect();

        tracker.store.add_snapshot(
            "a".to_string(),
            vec![
                make_fp("f.py", "foo", shared.clone()),
                make_fp("f.py", "bar", shared.clone()),
                make_fp("f.py", "baz", unrelated_a),
            ],
        );
        tracker.store.add_snapshot(
            "b".to_string(),
            vec![
                make_fp("f.py", "foo", shared),
                make_fp("f.py", "bar", diverged),
                make_fp("f.py", "baz", unrelated_b),
            ],
        );

        let diverging = tracker.detect_diverging_clones("a", "b");
        assert_eq!(diverging.len(), 1);
        // The diverging pair should be foo and bar.
        let pair = &diverging[0].clone_pair;
        let combined = format!("{} {}", pair.0, pair.1);
        assert!(
            combined.contains("foo") && combined.contains("bar"),
            "Expected foo and bar in diverging pair, got: {combined}"
        );
    }

    #[test]
    fn test_track_evolution_missing_snapshot() {
        let tracker = CloneEvolutionTracker::new(0.5);
        let events = tracker.track_evolution("nonexistent_a", "nonexistent_b");
        assert!(events.is_empty());
    }

    #[test]
    fn test_empty_snapshots() {
        let mut tracker = CloneEvolutionTracker::new(0.5);
        tracker.store.add_snapshot("a".to_string(), vec![]);
        tracker.store.add_snapshot("b".to_string(), vec![]);

        let events = tracker.track_evolution("a", "b");
        assert!(events.is_empty());
    }
}
