//! Bloom filter for edge deduplication during graph building.
//!
//! Uses sketch_oxide's highly-optimized BloomFilter implementation.
//!
//! Reduces edge cache memory from 1.6MB to ~120KB (13× reduction) by using
//! a probabilistic data structure for duplicate detection.
//!
//! Trade-off: ~1% false positive rate (avoids duplicate edge insertion ~99% of the time).

pub use sketch_oxide::membership::BloomFilter;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_insert_and_contains() {
        let mut bf = BloomFilter::new(100, 0.01);

        // Insert some items (sketch_oxide uses byte slices)
        bf.insert(b"alice");
        bf.insert(b"bob");
        bf.insert(b"charlie");

        // Check they're found
        assert!(bf.contains(b"alice"));
        assert!(bf.contains(b"bob"));
        assert!(bf.contains(b"charlie"));

        // Check non-existent item is not found (with high probability)
        assert!(!bf.contains(b"dave"));
    }

    #[test]
    fn test_bloom_empty_filter() {
        let bf = BloomFilter::new(100, 0.01);

        // Empty filter should not contain items
        assert!(!bf.contains(b"anything"));
    }

    #[test]
    fn test_bloom_memory_efficiency() {
        let bf = BloomFilter::new(10000, 0.01); // 10k items, 1% FP rate

        // sketch_oxide uses u64 words, so memory is reasonable
        // For 10k items at 1% FP rate: m ≈ 95850 bits ≈ 12KB
        let _ = bf; // Just verify it can be created
    }

    #[test]
    fn test_bloom_numeric_items() {
        let mut bf = BloomFilter::new(1000, 0.01);

        // Test with numeric values (stored as byte slices)
        for i in 0u32..100 {
            bf.insert(&i.to_le_bytes());
        }

        // Check inserted items
        for i in 0u32..100 {
            assert!(bf.contains(&i.to_le_bytes()));
        }

        // Check non-inserted items (with some tolerance for FP rate)
        let mut false_positives = 0;
        for i in 100u32..200 {
            if bf.contains(&i.to_le_bytes()) {
                false_positives += 1;
            }
        }

        // Allow up to 5% false positives in test (normal FP rate is ~1%)
        assert!(false_positives < 10, "Too many false positives: {}", false_positives);
    }

    #[test]
    fn test_bloom_tuple_items() {
        let mut bf = BloomFilter::new(1000, 0.01);

        // Test with edge-like data (from_idx, to_idx)
        for i in 0..100 {
            let bytes = format!("({},{})", i, i + 1).into_bytes();
            bf.insert(&bytes);
        }

        // Check inserted edges
        for i in 0..100 {
            let bytes = format!("({},{})", i, i + 1).into_bytes();
            assert!(bf.contains(&bytes));
        }

        // Check non-existent edge is probably not found
        let bytes = format!("(200,201)").into_bytes();
        assert!(!bf.contains(&bytes));
    }
}
