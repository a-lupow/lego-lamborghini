use log::{Metadata, Record};
use std::time::Instant;

pub struct SimpleLogger {
    start: Instant,
}

impl SimpleLogger {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let elapsed = self.start.elapsed();
            let ms = elapsed.as_millis();
            println!("[{:>10}ms] {} - {}", ms, record.level(), record.args());
        }
    }

    fn flush(&self) {}
}
