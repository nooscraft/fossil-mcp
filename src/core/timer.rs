//! Simple timing utility for performance tracing.

use std::time::{Duration, Instant};

/// Tracks elapsed time for a named operation.
#[derive(Debug)]
pub struct Timer {
    name: String,
    start: Instant,
    #[allow(dead_code)]
    parent: Option<String>,
}

impl Timer {
    /// Create a new timer for an operation.
    pub fn start(name: impl Into<String>) -> Self {
        let name = name.into();
        let start = Instant::now();
        eprintln!("[TRACE] Starting: {}", name);
        Self {
            name,
            start,
            parent: None,
        }
    }

    /// Create a nested timer with parent context.
    pub fn start_nested(name: impl Into<String>, parent: impl Into<String>) -> Self {
        let name = name.into();
        let parent = parent.into();
        let start = Instant::now();
        eprintln!("[TRACE]   └─ Starting: {}", name);
        Self {
            name,
            start,
            parent: Some(parent),
        }
    }

    /// Stop the timer and return elapsed duration.
    pub fn stop(self) -> Duration {
        let elapsed = self.start.elapsed();
        let ms = elapsed.as_millis();
        let s = elapsed.as_secs_f64();

        if ms < 1000 {
            eprintln!("[TRACE] Completed: {} in {}ms", self.name, ms);
        } else {
            eprintln!("[TRACE] Completed: {} in {:.2}s", self.name, s);
        }

        elapsed
    }

    /// Stop and log with additional info (e.g., count of items processed).
    pub fn stop_with_info(self, info: impl std::fmt::Display) -> Duration {
        let elapsed = self.start.elapsed();
        let ms = elapsed.as_millis();
        let s = elapsed.as_secs_f64();

        if ms < 1000 {
            eprintln!("[TRACE] Completed: {} ({}) in {}ms", self.name, info, ms);
        } else {
            eprintln!("[TRACE] Completed: {} ({}) in {:.2}s", self.name, info, s);
        }

        elapsed
    }

    /// Get current elapsed duration without stopping.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timer_basic() {
        let timer = Timer::start("test_operation");
        std::thread::sleep(Duration::from_millis(10));
        let elapsed = timer.stop();
        assert!(elapsed.as_millis() >= 10);
    }
}
