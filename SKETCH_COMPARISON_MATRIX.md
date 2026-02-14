# Sketch-Based Data Structures: Detailed Comparison Matrix

**Document Purpose:** Technical decision matrix for selecting probabilistic data structures for Fossil CodeGraph optimization.

---

## 1. BLOOM FILTER vs. ALTERNATIVES (Membership Checking)

### Bloom Filter Details

**Best for:** "Is edge A→B in the graph?" / "Was this function seen?"

| Aspect | Value | Notes |
|--------|-------|-------|
| **Space per element** | 9.6 bits (1% FP) | Tunable: k=5, m=1.2n |
| **Insertion time** | O(k) | k≈5-7 hash functions |
| **Query time** | O(k) | Same as insertion |
| **False positive rate** | 1% (tunable) | p = (1-(1-1/m)^kn)^k |
| **False negative rate** | 0% | Guaranteed |
| **Mergeable** | Yes | Bitwise OR of two filters |
| **Updatable** | No | Add-only structure |
| **Enumerable** | No | Cannot list members |

**Rust Implementation Example:**
```rust
use bloom_rs::BloomFilter;

let mut filter = BloomFilter::new(10_000, 0.01); // 10k items, 1% FP
filter.insert("edge:foo→bar");
filter.insert("edge:bar→baz");

// Query
if filter.contains("edge:foo→bar") {
    println!("Maybe exists (could be false positive)");
}
```

**Memory calculation:**
```
n = 100,000 edges
p = 0.01 (1% false positive rate)
m = -n * ln(p) / (ln(2)²) = -100k * (-4.605) / 0.4805 = 958,505 bits ≈ 120 KB
k = m * ln(2) / n ≈ 6 hash functions

Savings vs. HashSet<(NodeId, NodeId)>:
  HashSet: 100k × 16 bytes (two u64s) = 1.6 MB
  Bloom filter: 120 KB
  Ratio: 13× savings
```

### Comparison with Alternatives

| Structure | Space | Query | FP Rate | FN Rate | Mergeable | Use Case |
|-----------|-------|-------|---------|---------|-----------|----------|
| **Bloom Filter** | O(n) bits | O(k) | p(1%) | 0% | Yes | Membership |
| HashSet | O(n) words | O(1) avg | 0% | 0% | Yes | Exact set |
| Bit vector | O(n) bits | O(1) | 0% | 0% | Yes | Flags |
| Counting Bloom | O(4n) bits | O(k) | p(1%) | 0% | No | Deletions |
| Cuckoo Filter | O(n) bits | O(1) | p(0.5%) | 0% | No | Faster |
| Quotient Filter | O(n) bits | O(1) | p(0.5%) | 0% | Yes | Cache-friendly |

**Recommendation for Fossil:**
- Use Bloom filter if edge lookup is frequent hot path
- Trade 1% false positives for 13× memory savings
- Requires fallback to exact adjacency list for "maybe" results
- Prototype on clone detection hot loop first

---

## 2. COUNTMIN SKETCH vs. ALTERNATIVES (Frequency Estimation)

### CountMin Sketch Details

**Best for:** "How many times was function foo() called?" / "What are top N called functions?"

| Aspect | Value | Notes |
|--------|-------|-------|
| **Space** | O((1/ε) × ln(1/δ)) | ε=error margin, δ=failure prob |
| **For ε=1%, δ=1%** | ~1.4 KB | Fixed overhead |
| **Update time** | O(d) | d=depth, typically 4-6 |
| **Query time** | O(d) | Constant w.r.t. # of items |
| **Overestimation** | count(x) ≤ est(x) ≤ count(x) + ε×N | Guaranteed bound |
| **Underestimation** | Never | Always >= true count |
| **Mergeable** | Yes | Element-wise max of sketches |
| **Updatable** | Yes | Increment with decrement support |
| **Enumerable** | No | Cannot list heavy hitters directly |

**Rust Implementation Example:**
```rust
struct CountMinSketch {
    width: usize,  // w = ceil(e / ε)
    depth: usize,  // d = ceil(ln(1/δ))
    matrix: Vec<Vec<u32>>,
}

impl CountMinSketch {
    fn new(epsilon: f64, delta: f64) -> Self {
        let width = ((std::f64::consts::E / epsilon).ceil()) as usize;
        let depth = ((1.0 / delta).ln().ceil()) as usize;
        // ...
    }

    fn add(&mut self, item: &str, count: u32) {
        for row in 0..self.depth {
            let col = hash(row, item) % self.width;
            self.matrix[row][col] += count;
        }
    }

    fn estimate(&self, item: &str) -> u32 {
        (0..self.depth)
            .map(|row| {
                let col = hash(row, item) % self.width;
                self.matrix[row][col]
            })
            .min()
            .unwrap_or(0)
    }
}
```

**Memory calculation:**
```
Scenario: Track frequencies of function calls in large program
N items processed: 10 billion calls
ε = 1% (1% error margin)
δ = 1% (99% confidence)

width (w) = ceil(e / 0.01) = ceil(271.8) = 272
depth (d) = ceil(ln(1/0.01)) = ceil(4.605) = 5
entries = 272 × 5 = 1,360 (assuming 32-bit counters)
size = 1,360 × 4 = 5.44 KB

Savings vs. HashMap<String, u64>:
  Exact: 1 million unique calls × (32 bytes key + 8 bytes value) = 40 MB
  CMS: 5.44 KB
  Ratio: 7,350× savings (!!)

Error bound: estimate(foo_called) ≤ true_count(foo_called) + 0.01 × 10B
           = true_count + 100 million (worst case, unlikely)
```

### Comparison with Alternatives

| Structure | Space | Query | FP/Error | Streaming | Heap |
|-----------|-------|-------|----------|-----------|------|
| **Count-Min Sketch** | O((1/ε) ln(1/δ)) | O(d) | ε×N overest | Yes | O(d) |
| HashMap | O(n) | O(1) avg | 0 | No | O(n) |
| Frequency Array | O(n) | O(1) | 0 | No | O(n) |
| HeavyKeeper | O(1/ε) | O(d) | <ε×N | Yes | O(d) |
| Sketch-ML | O((1/ε) ln(1/δ)) | O(d) | <ε×N + log | Yes | O(d) |

**Recommendation for Fossil:**
- Use CMS for dynamic profiling (stream call events)
- Excellent for "top 100 functions" queries on streaming data
- Not suitable for exact dead code detection (overestimation)
- Prototype on optional runtime instrumentation layer

---

## 3. HYPERLOGLOG vs. ALTERNATIVES (Cardinality Estimation)

### HyperLogLog Details

**Best for:** "How many distinct functions are called?" / "How many unique types in signatures?"

| Aspect | Value | Notes |
|--------|-------|-------|
| **Space** | O(log log n) | ~1.5 KB for m=16k registers |
| **Standard error** | 1.04/√m | ~0.8% for m=16k |
| **Add time** | O(1) | Hash + bit operations |
| **Query time** | O(m) | Must scan all registers |
| **Cardinality range** | 0 to 2^32 (billions) | Logarithmic scaling |
| **Mergeable** | Yes | Element-wise max of register arrays |
| **Updatable** | Yes | Update register if max increases |
| **Enumerable** | No | Cannot list distinct items |
| **Bias** | Small (small cardinalities <100) | Correctable with empirical bias table |

**Rust Implementation Example:**
```rust
struct HyperLogLog {
    registers: Vec<u8>,  // m = 2^p registers (typically p=14 → m=16384)
    p: usize,
}

impl HyperLogLog {
    fn new(precision: usize) -> Self {
        let m = 1 << precision;
        HyperLogLog {
            registers: vec![0; m],
            p: precision,
        }
    }

    fn add(&mut self, item: &str) {
        let hash = hash(item);
        let j = (hash >> (64 - self.p)) as usize; // First p bits
        let leading_zeros = (hash << self.p).leading_zeros();
        self.registers[j] = self.registers[j].max(leading_zeros as u8);
    }

    fn count(&self) -> f64 {
        let m = 1 << self.p;
        let alpha = match m {
            16 => 0.673,
            32 => 0.697,
            64 => 0.709,
            _ => 0.7213 / (1.0 + 1.079 / m as f64),
        };
        let raw_estimate = alpha * m as f64 * m as f64
            / self.registers.iter().map(|&r| 2.0_f64.powi(-(r as i32))).sum::<f64>();

        // Apply small-range and large-range corrections
        if raw_estimate <= 2.5 * m as f64 {
            // Small range correction
            let empty = self.registers.iter().filter(|&&r| r == 0).count();
            if empty > 0 {
                return m as f64 * (m as f64 / empty as f64).ln();
            }
        }

        if raw_estimate <= (1.0/30.0) * (1u64 << 32) as f64 {
            raw_estimate
        } else {
            -((1u64 << 32) as f64) * (1.0 - raw_estimate / (1u64 << 32) as f64).ln()
        }
    }
}
```

**Memory calculation:**
```
Scenario: Estimate distinct functions in large codebase
True cardinality: 5 million unique functions
Precision (p): 14 (typical)
Registers: 2^14 = 16,384

Size = 16,384 bytes = 16 KB

Comparison with exact HashSet:
  HashSet: 5 million × 8 bytes (hashes) + overhead = ~40-60 MB
  HyperLogLog: 16 KB
  Ratio: 2,500-3,750× savings

Accuracy:
  Standard error: 1.04 / sqrt(16384) ≈ 0.008 = 0.8%
  Estimated count: 5 million ± 40,000 (with 95% confidence)
  Relative error: 0.8%
```

### Comparison with Alternatives

| Structure | Space | Accuracy | Query | Streaming | Mergeable |
|-----------|-------|----------|-------|-----------|-----------|
| **HyperLogLog** | O(log log n) | 1.04/√m | O(m) | Yes | Yes |
| HashSet | O(n) | 0% | O(1) avg | No | Yes |
| Exact counter | O(1) | 0% | O(1) | No | No |
| Min-Values | O(k) | √(1-1/k) | O(k) | Yes | Yes |
| Adaptive CMS | O(k) | 1/k | O(k) | Yes | Yes |
| Probabilistic | O(log n) | Variable | O(1-k) | Maybe | Maybe |

**Recommendation for Fossil:**
- Use HLL for statistics reporting ("~5M distinct functions")
- Excellent for benchmarking and capacity planning
- Not suitable for threshold-based decisions (2% error can matter)
- Prototype in CLI stats output (lowest risk integration)

---

## 4. MINHASH vs. ALTERNATIVES (Similarity Detection)

### MinHash Details

**Best for:** Clone detection, code similarity estimation, refactoring candidates

| Aspect | Value | Notes |
|--------|-------|-------|
| **Signatures per item** | k hashes (64-256) | Trade accuracy for speed |
| **Space per item** | k × 4-8 bytes | 256 hashes = 1-2 KB per function |
| **Comparison time** | O(k) | Hash value comparison |
| **Similarity measure** | Jaccard index | intersection / union |
| **Accuracy** | ~99.99% (k=128) | P(miss) ≈ (1-sim)^k |
| **False positives** | Tunable | Higher if threshold too low |
| **False negatives** | ~(1-similarity)^k | Exponentially decreases with k |
| **Mergeable** | Yes | Combine signatures from splits |
| **Enumerable** | Yes | Can extract related items |

**Fossil Already Uses:** MinHash is implemented in clone detection (per MEMORY.md)

**Enhanced LSH Application:**
```rust
struct LSHBands {
    num_bands: usize,
    rows_per_band: usize,
    bands: Vec<HashMap<Vec<u32>, Vec<usize>>>,
}

// Instead of comparing all pairs (O(n²)):
// 1. Hash each MinHash signature into b bands
// 2. Each band produces a hash of r hash values
// 3. Functions in same band bucket → candidates
// 4. Compare only candidates (10-100× speedup)

impl LSHBands {
    fn hash_function_to_bands(&mut self, func_id: usize, signatures: &[u32]) {
        for band_idx in 0..self.num_bands {
            let start = band_idx * self.rows_per_band;
            let end = (start + self.rows_per_band).min(signatures.len());
            let band_hash = hash(&signatures[start..end]);
            self.bands[band_idx]
                .entry(band_hash)
                .or_insert_with(Vec::new)
                .push(func_id);
        }
    }

    fn get_candidates(&self, func_id: usize, signatures: &[u32]) -> HashSet<usize> {
        let mut candidates = HashSet::new();
        for band_idx in 0..self.num_bands {
            let start = band_idx * self.rows_per_band;
            let end = (start + self.rows_per_band).min(signatures.len());
            let band_hash = hash(&signatures[start..end]);
            if let Some(funcs) = self.bands[band_idx].get(&band_hash) {
                for &f in funcs {
                    if f != func_id {
                        candidates.insert(f);
                    }
                }
            }
        }
        candidates
    }
}
```

**Memory calculation (LSH optimization):**
```
Scenario: Detect clones in 100,000 functions
MinHash signatures: k=128 per function (512 bytes each)
LSH parameters: b=10 bands, r=13 rows per band (128 = 10*13)

Traditional MinHash:
  Pairwise comparisons: C(100k, 2) = 5 billion comparisons
  Time: 5B × O(128) = 640 billion hash comparisons
  Space: 100k × 512 bytes = 51.2 MB

LSH-optimized:
  Time: 100k hashes into bands + candidate comparisons
  Expected candidates per function: ~(threshold)^r functions
    For similarity threshold 0.8: ~0.8^13 × 100k = 1-2 functions (typically)
  Time: 100k × (band hashing) + 100k × (1-2 comparisons)
  Speedup: ~1000× (!!)
```

### Comparison with Alternatives

| Method | Comparison Time | Space | Accuracy | Notes |
|--------|-----------------|-------|----------|-------|
| **MinHash** | O(k) | O(k) | ~99% (k=128) | Industry standard |
| **Exact Jaccard** | O(n) | O(n) | 100% | For small items only |
| **Simhash** | O(n) | O(n) | 95% | Better for documents |
| **Fuzzy Hash** | O(n) | O(n) | Variable | Binary data focus |
| **AST/Semantic** | O(n²) nodes | O(n) | 99%+ | Slower, more precise |
| **Token Sequence** | O(n) | O(n) | 90% | Ignores structure |
| **Edit Distance** | O(n²) | O(1) | 100% | Very slow |

**Recommendation for Fossil:**
- Fossil already uses MinHash effectively
- Next step: Add LSH banding for O(n²) → O(n) speedup
- Prototype LSH on 10k+ function codebases (where O(n²) becomes bottleneck)
- Keep current threshold=0.8; LSH parameters: b=10, r=13

---

## 5. ACADEMIC TECHNIQUES COMPARISON

### Field-Based vs. Context-Sensitive Analysis

| Aspect | Field-Based | Context-Sensitive | Notes |
|--------|-------------|-------------------|-------|
| **Memory** | 10% (800 MB for 1M LOC) | 100% (4 GB baseline) | 5× savings |
| **Speed** | 50% faster | Baseline | Fewer context nodes |
| **Precision** | 95% (JS aliases) | 99%+ | Few false negatives |
| **Implementation** | Custom alias pass | Context manager | Fossil: not implemented |
| **Scalability** | Linear | O(contexts) | For 1M LOC: 20k contexts |

**For Fossil:**
- Current: Language-specific, flow-insensitive extraction (per parser)
- Potential: Field-based alias tracking for JavaScript imports
- Effort: Medium (custom alias analysis module)
- Benefit: Reduce import resolver complexity by 30-50%

### Demand-Driven Analysis

| Aspect | Demand-Driven | Exhaustive | Notes |
|--------|---------------|-----------|-------|
| **Memory** | O(reachable) | O(all) | Saves 60-80% for sparse graphs |
| **Speed** | Slower per query | Faster overall | But answers specific queries faster |
| **Scope** | Answerable sets | Full knowledge | Fossil: Currently exhaustive |
| **Implementation** | Lazy evaluation | Eager computation | Requires restructuring |

**For Fossil:**
- Current: Compute full CodeGraph, then analyze
- Potential: Lazy entry point discovery (only analyze reachable from entries)
- Effort: High (redesign analysis pipeline)
- Benefit: For large projects with sparse reachability, save 60-80% work

---

## Implementation Complexity Matrix

```
             MEMORY SAVINGS    IMPLEMENTATION    ACCURACY IMPACT    RISK

HyperLogLog  Medium (50-80%)   Low (1 day)       None (stats only)  Very Low
             for large projects

Bloom Filter Medium-High        Low (1-2 days)    Low (1% FP)        Low
             (10-20× for index)

CMS          Very High (99%+)   Medium (2-3 days) Medium (oversized) Medium
             for frequencies

MinHash+LSH  Very High (10×)    Medium (2-3 days) None (faster ver)  Low
             for clone speed

Field-based  High (5× for JS)   High (5-7 days)   Low (95% acc)      Medium
             JS/TS resolution

Context-sel. High (5× general)  Very High (1+ wks) Very Low           High
             memory savings
```

---

## Recommended Implementation Sequence

### Week 1: Foundation (Lowest Risk)
1. **HyperLogLog stats** (1-2 hours of coding, testing)
   - Add to CLI: `fossil dead-code --stats` shows cardinality estimates
   - Zero behavior change, pure enhancement
   - Validates probabilistic data structure integration

2. **Bloom Filter prototype** (4-6 hours, optional)
   - Create `edge_filter: Option<BloomFilter>` in CodeGraph
   - Benchmark against real projects
   - Measure: false positive rate, actual memory savings
   - Decide: worth enabling?

### Week 2-3: Optimization (Medium Effort)
3. **LSH for clone detection** (8-16 hours)
   - Add LSHBands wrapper around existing MinHash signatures
   - Band parameters: b=10, r=13 (for 128-hash signatures)
   - Benchmark: 1000× speedup expected on 100k+ functions
   - Output: same results, faster

4. **CMS for profiling** (optional, 2-3 days)
   - Create optional `--profile-calls` mode
   - Stream function calls through CMS
   - Output: frequency distribution with error bounds
   - Use case: identify hot paths for optimization

### Months 2-3: Strategic (High Complexity)
5. **Field-based JS/TS import resolution** (effort TBD)
   - Custom alias analysis for imported names
   - Reduce import resolver work by 30-50%
   - Requires prototype testing on real codebases

---

## Decision Framework

**Choose Bloom Filter IF:**
- Edge existence queries are hot path (profiler confirms)
- False positive rate <2% acceptable
- Can maintain exact adjacency list for "maybe" cases

**Choose CountMin Sketch IF:**
- Tracking call frequencies at scale (>100M calls)
- 1-5% overestimation acceptable
- Need constant space for streaming data

**Choose HyperLogLog IF:**
- Need cardinality statistics for reporting
- 2% error acceptable for metrics
- Want to display "~5M functions" instead of exact count

**Choose MinHash+LSH IF:**
- Clone detection bottleneck confirmed (O(n²) hot)
- Have >10,000 functions in codebase
- Current clone detection speed insufficient

**Choose Field-Based Analysis IF:**
- JavaScript/TypeScript projects dominate workload
- Import resolution is profiled bottleneck
- 5% false negatives acceptable for complex aliasing

---

**End of Comparison Document**
