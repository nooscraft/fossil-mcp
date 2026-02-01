//! Clone clustering via Union-Find.
//!
//! Groups related clone groups into higher-level `CloneClass` clusters.
//! Two clone groups are related when they share at least one overlapping
//! file + line range in any of their instances.

use std::cmp::Ordering;
use std::collections::HashMap;

use super::types::{CloneGroup, CloneInstance};

// ===========================================================================
// Union-Find
// ===========================================================================

/// Union-Find (disjoint set) with path compression and union by rank.
pub struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    pub fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    pub fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]); // path compression
        }
        self.parent[x]
    }

    pub fn union(&mut self, x: usize, y: usize) {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx == ry {
            return;
        }
        match self.rank[rx].cmp(&self.rank[ry]) {
            Ordering::Less => self.parent[rx] = ry,
            Ordering::Greater => self.parent[ry] = rx,
            Ordering::Equal => {
                self.parent[ry] = rx;
                self.rank[rx] += 1;
            }
        }
    }

    pub fn connected(&mut self, x: usize, y: usize) -> bool {
        self.find(x) == self.find(y)
    }

    /// Returns all clusters as `Vec<Vec<usize>>`.
    pub fn clusters(&mut self) -> Vec<Vec<usize>> {
        let n = self.parent.len();
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..n {
            let root = self.find(i);
            groups.entry(root).or_default().push(i);
        }
        groups.into_values().collect()
    }
}

// ===========================================================================
// CloneClass
// ===========================================================================

/// A cluster of related clone groups.
#[derive(Debug, Clone)]
pub struct CloneClass {
    pub id: usize,
    pub groups: Vec<CloneGroup>,
    pub total_duplicated_lines: usize,
}

// ===========================================================================
// Overlap detection helper
// ===========================================================================

/// Check whether two clone instances overlap in the same file.
fn instances_overlap(a: &CloneInstance, b: &CloneInstance) -> bool {
    if a.file != b.file {
        return false;
    }
    // Lines overlap when neither is entirely before the other.
    a.start_line <= b.end_line && b.start_line <= a.end_line
}

/// Check whether two clone groups share at least one overlapping instance.
fn groups_overlap(a: &CloneGroup, b: &CloneGroup) -> bool {
    for ai in &a.instances {
        for bi in &b.instances {
            if instances_overlap(ai, bi) {
                return true;
            }
        }
    }
    false
}

// ===========================================================================
// Clustering entry point
// ===========================================================================

/// Cluster clone groups that share overlapping file+line ranges.
///
/// Two groups are placed in the same `CloneClass` when they (transitively)
/// share an overlapping `CloneInstance`.
pub fn cluster_clone_groups(groups: &[CloneGroup]) -> Vec<CloneClass> {
    let n = groups.len();
    if n == 0 {
        return Vec::new();
    }

    let mut uf = UnionFind::new(n);

    // Union every pair of groups that share an overlapping instance.
    for i in 0..n {
        for j in (i + 1)..n {
            if groups_overlap(&groups[i], &groups[j]) {
                uf.union(i, j);
            }
        }
    }

    // Build CloneClass objects from clusters.
    let clusters = uf.clusters();
    let mut classes: Vec<CloneClass> = clusters
        .into_iter()
        .enumerate()
        .map(|(id, member_indices)| {
            let cluster_groups: Vec<CloneGroup> = member_indices
                .iter()
                .map(|&idx| groups[idx].clone())
                .collect();
            let total_duplicated_lines: usize =
                cluster_groups.iter().map(|g| g.duplicated_lines()).sum();
            CloneClass {
                id,
                groups: cluster_groups,
                total_duplicated_lines,
            }
        })
        .collect();

    // Sort by total_duplicated_lines descending for deterministic, useful ordering.
    classes.sort_by(|a, b| b.total_duplicated_lines.cmp(&a.total_duplicated_lines));

    // Re-assign ids after sorting.
    for (i, cls) in classes.iter_mut().enumerate() {
        cls.id = i;
    }

    classes
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clones::types::{CloneGroup, CloneInstance, CloneType};

    // ---- helpers ----------------------------------------------------------

    fn make_instance(file: &str, start: usize, end: usize) -> CloneInstance {
        CloneInstance::new(file.to_string(), start, end)
    }

    fn make_group(instances: Vec<CloneInstance>) -> CloneGroup {
        CloneGroup::new(CloneType::Type1, instances)
    }

    // ---- UnionFind tests -------------------------------------------------

    #[test]
    fn test_union_find_basic() {
        let mut uf = UnionFind::new(5);
        assert!(!uf.connected(0, 1));
        uf.union(0, 1);
        assert!(uf.connected(0, 1));
        assert!(!uf.connected(0, 2));
    }

    #[test]
    fn test_union_find_transitive() {
        let mut uf = UnionFind::new(5);
        uf.union(0, 1);
        uf.union(1, 2);
        assert!(uf.connected(0, 2));
    }

    #[test]
    fn test_union_find_clusters() {
        let mut uf = UnionFind::new(6);
        uf.union(0, 1);
        uf.union(1, 2);
        uf.union(3, 4);
        // 5 is isolated

        let mut clusters = uf.clusters();
        // Sort clusters by first element for deterministic comparison.
        for c in &mut clusters {
            c.sort();
        }
        clusters.sort_by_key(|c| c[0]);

        assert_eq!(clusters.len(), 3);
        assert_eq!(clusters[0], vec![0, 1, 2]);
        assert_eq!(clusters[1], vec![3, 4]);
        assert_eq!(clusters[2], vec![5]);
    }

    #[test]
    fn test_union_find_self_union() {
        let mut uf = UnionFind::new(3);
        uf.union(1, 1);
        assert!(uf.connected(1, 1));
        // Should still have 3 clusters.
        assert_eq!(uf.clusters().len(), 3);
    }

    #[test]
    fn test_union_find_path_compression() {
        let mut uf = UnionFind::new(10);
        // Build a chain: 0-1-2-3-4
        for i in 0..4 {
            uf.union(i, i + 1);
        }
        // After find(4), the path should be compressed.
        let root = uf.find(4);
        // All should share the same root now.
        for i in 0..5 {
            assert_eq!(uf.find(i), root);
        }
    }

    // ---- instances_overlap tests -----------------------------------------

    #[test]
    fn test_instances_overlap_same_file() {
        let a = make_instance("foo.rs", 10, 20);
        let b = make_instance("foo.rs", 15, 25);
        assert!(instances_overlap(&a, &b));
    }

    #[test]
    fn test_instances_no_overlap_same_file() {
        let a = make_instance("foo.rs", 10, 20);
        let b = make_instance("foo.rs", 21, 30);
        assert!(!instances_overlap(&a, &b));
    }

    #[test]
    fn test_instances_no_overlap_different_file() {
        let a = make_instance("foo.rs", 10, 20);
        let b = make_instance("bar.rs", 10, 20);
        assert!(!instances_overlap(&a, &b));
    }

    // ---- cluster_clone_groups tests --------------------------------------

    #[test]
    fn test_cluster_empty() {
        let classes = cluster_clone_groups(&[]);
        assert!(classes.is_empty());
    }

    #[test]
    fn test_cluster_no_overlap() {
        let g1 = make_group(vec![
            make_instance("a.rs", 1, 10),
            make_instance("b.rs", 1, 10),
        ]);
        let g2 = make_group(vec![
            make_instance("c.rs", 1, 10),
            make_instance("d.rs", 1, 10),
        ]);
        let classes = cluster_clone_groups(&[g1, g2]);
        // No overlap -> each group in its own class.
        assert_eq!(classes.len(), 2);
    }

    #[test]
    fn test_cluster_with_overlap() {
        // g1 and g2 share overlapping ranges in "a.rs"
        let g1 = make_group(vec![
            make_instance("a.rs", 1, 10),
            make_instance("b.rs", 1, 10),
        ]);
        let g2 = make_group(vec![
            make_instance("a.rs", 5, 15),
            make_instance("c.rs", 1, 10),
        ]);
        let classes = cluster_clone_groups(&[g1, g2]);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].groups.len(), 2);
    }

    #[test]
    fn test_cluster_transitive_overlap() {
        // g1 overlaps with g2, g2 overlaps with g3 -> all in one class
        let g1 = make_group(vec![make_instance("a.rs", 1, 10)]);
        let g2 = make_group(vec![
            make_instance("a.rs", 5, 15),
            make_instance("b.rs", 1, 10),
        ]);
        let g3 = make_group(vec![make_instance("b.rs", 5, 15)]);

        let classes = cluster_clone_groups(&[g1, g2, g3]);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].groups.len(), 3);
    }

    #[test]
    fn test_cluster_duplicated_lines_sum() {
        // Each instance is 10 lines, 2 instances per group: duplicated = 10*(2-1) = 10
        let g1 = make_group(vec![
            make_instance("a.rs", 1, 10),
            make_instance("b.rs", 1, 10),
        ]);
        let g2 = make_group(vec![
            make_instance("a.rs", 5, 14),
            make_instance("c.rs", 5, 14),
        ]);
        let classes = cluster_clone_groups(&[g1, g2]);
        // They overlap in a.rs, so they are one class.
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].total_duplicated_lines, 20); // 10 + 10
    }

    #[test]
    fn test_cluster_single_group() {
        let g1 = make_group(vec![
            make_instance("a.rs", 1, 10),
            make_instance("b.rs", 1, 10),
        ]);
        let classes = cluster_clone_groups(&[g1]);
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].id, 0);
    }
}
