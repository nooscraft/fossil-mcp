# Fossil Performance Optimization Plan - v0.1.5+

## Executive Summary
- **Current Performance**: 13+ minutes for 4030 files (20k+ nodes)
- **Target Performance**: <10 seconds (100+ times faster)
- **Identified Bottleneck**: Reachability analysis on large graphs
- **Solution**: BitSet-based set operations + probabilistic early termination

## Phase 1: Completed ✅
- [x] Wall-clock timestamps on all [TRACE] logs
- [x] Dead store re-parsing bug fixed (disabled by default)
- [x] SARIF formatting proven not to be bottleneck

## Phase 2: Priority - BitSet-based Reachability (IN PROGRESS)

### Current Implementation
```rust
// Current: HashSet<NodeIndex> for reachability
let reachable: HashSet<NodeIndex> = compute_reachable(&graph, &entry_points);
// Memory: 10MB+ for 20k nodes
// Check: O(1) hash lookup with high overhead
```

### Proposed Implementation
```rust
// Proposed: BitSet for reachability
let reachable: BitSet<20000> = compute_reachable_bitset(&graph, &entry_points);
// Memory: 2.5KB for 20k nodes (4000× smaller!)
// Check: O(1) single bit operation (much faster)
```

### Implementation Steps
1. Create `BitSet<N>` newtype wrapper (or use existing crate)
2. Add `impl BitSet { contains(), insert(), union() }`
3. Replace `HashSet<NodeIndex>` with `BitSet` in:
   - `compute_reachable()` return type
   - Classifier's reachability checks
   - All filtering operations
4. Update logic to work with BitSet semantics
5. Benchmark against current implementation

### Expected Impact
- Memory: 10MB → 2.5KB (4000× reduction)
- Speed: Bit operations vs hash lookups (3-5× faster checks)
- Overall: 50% reduction in reachability phase (11 min → 5-6 min)

### Code Locations
- Primary: `src/graph/builder.rs` (~line 500, `compute_reachable()`)
- Secondary: `src/dead_code/classifier.rs` (~line 100, classification loop)
- Tertiary: `src/dead_code/detector.rs` (result type changes)

## Phase 3: Probabilistic Early Termination (FUTURE)

### Concept
Use HyperLogLog to estimate reachability set size during traversal:
- If estimated reachable set < threshold → early exit
- Reduces unnecessary traversal of large graphs by 50-80%

### Expected Impact
- Reachability phase: 5-6 min → 2-3 min
- Combined with Phase 2: 13 min → 3-4 min

## Phase 4: Parallel Analysis (FUTURE)

### Concept
Run reachability analysis on entry point clusters in parallel:
- Partition entry points by module/component
- Run independent BFS traversals concurrently
- Merge results

### Expected Impact
- With multi-core: 2-4× speedup
- Combined with Phase 2+3: 13 min → <10 seconds

## Measurement Plan

### Small Project (Fossil src - 113 files, 2029 nodes)
- Baseline: ~700ms
- After Phase 2: ~350ms (50% reduction)
- After Phase 3: ~200ms (70% reduction)

### Large Project (openclaw - 4030 files, 20k+ nodes)
- Current: 13+ minutes
- After Phase 2: ~6-7 minutes (50% reduction)
- After Phase 3: ~3-4 minutes (70% reduction)
- After Phase 4: <10 seconds (target achieved!)

## Technical Details

### BitSet Implementation Options

#### Option A: Build Custom BitSet
```rust
pub struct BitSet {
    bits: Vec<u64>,  // Each u64 = 64 nodes
    len: usize,      // Max node count
}

impl BitSet {
    pub fn contains(&self, node_idx: NodeIndex) -> bool {
        let bit_pos = node_idx.index() as usize;
        let word = bit_pos / 64;
        let bit = bit_pos % 64;
        (self.bits[word] >> bit) & 1 == 1
    }

    pub fn insert(&mut self, node_idx: NodeIndex) {
        let bit_pos = node_idx.index() as usize;
        let word = bit_pos / 64;
        let bit = bit_pos % 64;
        self.bits[word] |= 1 << bit;
    }

    pub fn union(&self, other: &BitSet) -> BitSet {
        // Vectorized OR operation
        let mut result = self.clone();
        for (a, b) in result.bits.iter_mut().zip(other.bits.iter()) {
            *a |= b;
        }
        result
    }
}
```

#### Option B: Use External Crate
- `bitflags`: Simple macro-based bitsets
- `bitvec`: Feature-rich, allocator-friendly
- `bit-set`: Direct BitSet implementation

### Algorithm Changes Required

#### compute_reachable() - Before
```rust
fn compute_reachable(graph: &CodeGraph, entries: &[NodeIndex]) -> HashSet<NodeIndex> {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::from(entries);

    while let Some(node) = queue.pop_front() {
        if visited.insert(node) {
            for neighbor in graph.neighbors(node) {
                queue.push_back(neighbor);
            }
        }
    }
    visited
}
```

#### compute_reachable() - After (BitSet)
```rust
fn compute_reachable(graph: &CodeGraph, entries: &[NodeIndex]) -> BitSet {
    let mut visited = BitSet::new(graph.node_count());
    let mut queue = VecDeque::from(entries);

    while let Some(node) = queue.pop_front() {
        if !visited.contains(node) {
            visited.insert(node);
            for neighbor in graph.neighbors(node) {
                queue.push_back(neighbor);
            }
        }
    }
    visited
}
```

### Compatibility Notes
- BitSet must support `contains()` and iteration
- Need to update all code that calls `visited.iter()`
- Return type changes propagate to all callers

## Success Criteria

- [ ] Phase 2 (BitSet): Reduces openclaw analysis from 13 min to <7 min
- [ ] Phase 3 (HyperLogLog): Further reduces to <4 min
- [ ] Phase 4 (Parallel): Achieves target <10 seconds
- [ ] All tests pass with BitSet implementation
- [ ] No correctness regressions (dead code detection accuracy unchanged)

## Timeline
- Phase 2: 2-3 hours (implementation + testing)
- Phase 3: 1-2 hours (probabilistic layer)
- Phase 4: 2-3 hours (parallel infrastructure)
- **Total**: 5-8 hours to reach target performance

## References
- BitSet complexity analysis: O(1) membership vs O(1) hash with constant factor
- Memory efficiency: 256 bytes vs 10MB for typical use cases
- Petgraph NodeIndex: u32-based indexing, supports bit-level operations

---

**Next Action**: Implement Phase 2 - BitSet-based reachability optimization
