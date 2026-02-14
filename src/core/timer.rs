//! Simple timing utility for performance tracing.

use std::time::{Duration, Instant, SystemTime};

/// Get current wall-clock time as ISO 8601 string
fn current_time_str() -> String {
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => {
            let secs = duration.as_secs();
            let nanos = duration.subsec_millis();
            format!("{:05}.{:03}", secs % 100000, nanos)
        }
        Err(_) => "??:??:??".to_string(),
    }
}

/// Tracks elapsed time for a named operation.
#[derive(Debug)]
pub struct Timer {
    name: String,
    start: Instant,
    start_time: String,
    #[allow(dead_code)]
    parent: Option<String>,
}

impl Timer {
    /// Create a new timer for an operation.
    pub fn start(name: impl Into<String>) -> Self {
        let name = name.into();
        let start = Instant::now();
        let start_time = current_time_str();
        eprintln!("[TRACE] [{}] Starting: {}", start_time, name);
        Self {
            name,
            start,
            start_time,
            parent: None,
        }
    }

    /// Create a nested timer with parent context.
    pub fn start_nested(name: impl Into<String>, parent: impl Into<String>) -> Self {
        let name = name.into();
        let parent = parent.into();
        let start = Instant::now();
        let start_time = current_time_str();
        eprintln!("[TRACE] [{}]   └─ Starting: {}", start_time, name);
        Self {
            name,
            start,
            start_time,
            parent: Some(parent),
        }
    }

    /// Stop the timer and return elapsed duration.
    pub fn stop(self) -> Duration {
        let elapsed = self.start.elapsed();
        let end_time = current_time_str();
        let ms = elapsed.as_millis();
        let s = elapsed.as_secs_f64();

        if ms < 1000 {
            eprintln!("[TRACE] [{}] Completed: {} in {}ms", end_time, self.name, ms);
        } else {
            eprintln!("[TRACE] [{}] Completed: {} in {:.2}s", end_time, self.name, s);
        }

        elapsed
    }

    /// Stop and log with additional info (e.g., count of items processed).
    pub fn stop_with_info(self, info: impl std::fmt::Display) -> Duration {
        let elapsed = self.start.elapsed();
        let end_time = current_time_str();
        let ms = elapsed.as_millis();
        let s = elapsed.as_secs_f64();

        if ms < 1000 {
            eprintln!("[TRACE] [{}] Completed: {} ({}) in {}ms", end_time, self.name, info, ms);
        } else {
            eprintln!("[TRACE] [{}] Completed: {} ({}) in {:.2}s", end_time, self.name, info, s);
        }

        elapsed
    }

    /// Get current elapsed duration without stopping.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Log a trace message with current timestamp and optional elapsed time.
pub fn trace_msg(msg: impl std::fmt::Display) {
    let time_str = current_time_str();
    eprintln!("[TRACE] [{}] {}", time_str, msg);
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
