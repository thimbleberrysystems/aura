use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct AppContext {
    start: Instant,
}

impl AppContext {
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }

    pub fn uptime_seconds(&self) -> u64 {
        let d: Duration = Instant::now().saturating_duration_since(self.start);
        d.as_secs()
    }
}
