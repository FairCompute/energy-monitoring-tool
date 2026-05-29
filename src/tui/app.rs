use crate::monitor::{MetricsSnapshot, MonitorHandle};
use std::time::Instant;

pub struct App {
    handle: MonitorHandle,
    start_time: Instant,
    pub should_quit: bool,
}

impl App {
    pub fn new(handle: MonitorHandle) -> Self {
        Self {
            handle,
            start_time: Instant::now(),
            should_quit: false,
        }
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        self.handle.snapshot()
    }

    pub fn uptime_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}
