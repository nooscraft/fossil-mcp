//! Stress test command for measuring performance on large projects.

use std::path::Path;
use crate::analysis::StressTestRunner;

pub fn run(path: &Path, compare: bool) -> Result<String, crate::core::Error> {
    if compare {
        // Run comparative analysis
        let comparison = StressTestRunner::compare_approaches(path)?;
        Ok(comparison)
    } else {
        // Run full pipeline stress test
        let result = StressTestRunner::run_full_pipeline(path)?;
        Ok(result.summary())
    }
}
