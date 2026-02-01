//! Text utilities for efficient source code position mapping.
//!
//! The [`LineOffsetTable`] provides O(log n) byte-position to line/column conversion
//! by pre-computing line start offsets.

/// Pre-computed table of line start byte offsets for O(log n) lookups.
#[derive(Debug, Clone)]
pub struct LineOffsetTable {
    /// Byte offset of the start of each line (0-indexed lines).
    /// `offsets[0]` is always 0.
    offsets: Vec<usize>,
}

impl LineOffsetTable {
    /// Build the offset table from source text. O(n) construction.
    pub fn new(source: &str) -> Self {
        let mut offsets = vec![0];
        for (i, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                offsets.push(i + 1);
            }
        }
        Self { offsets }
    }

    /// Convert a byte offset to (line, column), both 0-indexed. O(log n).
    pub fn byte_to_line_col(&self, byte_offset: usize) -> (usize, usize) {
        let line = match self.offsets.binary_search(&byte_offset) {
            Ok(exact) => exact,
            Err(insert) => insert.saturating_sub(1),
        };
        let col = byte_offset.saturating_sub(self.offsets[line]);
        (line, col)
    }

    /// Convert a byte offset to 1-indexed line number. O(log n).
    pub fn byte_to_line1(&self, byte_offset: usize) -> usize {
        self.byte_to_line_col(byte_offset).0 + 1
    }

    /// Get the byte offset for the start of a 0-indexed line.
    pub fn line_start(&self, line: usize) -> Option<usize> {
        self.offsets.get(line).copied()
    }

    /// Total number of lines.
    pub fn line_count(&self) -> usize {
        self.offsets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_source() {
        let table = LineOffsetTable::new("");
        assert_eq!(table.line_count(), 1);
        assert_eq!(table.byte_to_line_col(0), (0, 0));
    }

    #[test]
    fn test_single_line() {
        let table = LineOffsetTable::new("hello world");
        assert_eq!(table.line_count(), 1);
        assert_eq!(table.byte_to_line_col(0), (0, 0));
        assert_eq!(table.byte_to_line_col(5), (0, 5));
    }

    #[test]
    fn test_multi_line() {
        let source = "line one\nline two\nline three";
        let table = LineOffsetTable::new(source);
        assert_eq!(table.line_count(), 3);

        // Start of line 0
        assert_eq!(table.byte_to_line_col(0), (0, 0));
        // End of line 0 (the 'e' in "one")
        assert_eq!(table.byte_to_line_col(7), (0, 7));
        // Start of line 1 (byte after '\n')
        assert_eq!(table.byte_to_line_col(9), (1, 0));
        // Start of line 2
        assert_eq!(table.byte_to_line_col(18), (2, 0));
    }

    #[test]
    fn test_line1_indexing() {
        let source = "a\nb\nc";
        let table = LineOffsetTable::new(source);
        assert_eq!(table.byte_to_line1(0), 1);
        assert_eq!(table.byte_to_line1(2), 2);
        assert_eq!(table.byte_to_line1(4), 3);
    }

    #[test]
    fn test_line_start() {
        let source = "abc\ndef\nghi";
        let table = LineOffsetTable::new(source);
        assert_eq!(table.line_start(0), Some(0));
        assert_eq!(table.line_start(1), Some(4));
        assert_eq!(table.line_start(2), Some(8));
        assert_eq!(table.line_start(3), None);
    }
}
