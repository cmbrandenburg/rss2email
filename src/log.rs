use std;
use std::io::Write;

#[derive(Debug)]
pub struct Logger {
    level: LogLevel,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LogLevel {
    #[cfg(test)]
    Nothing,
    Important,
    Normal,
    Verbose,
}

#[derive(Clone, Copy, Debug)]
pub enum LogKind {
    Error,
    #[allow(unused)]
    Warning,
    Info,
}

impl Logger {
    pub fn new(level: LogLevel) -> Self {
        Logger { level: level }
    }

    pub fn log<M: std::fmt::Display>(&self, level: LogLevel, kind: LogKind, message: M) {

        if level > self.level {
            return; // ignore this message
        }

        let prefix = match kind {
            LogKind::Error => "*** ",
            LogKind::Warning => "!!! ",
            LogKind::Info => "",
        };

        let actual = format!("{}{}\n", prefix, message.to_string());

        let stderr = std::io::stderr();
        stderr.lock().write_all(actual.as_bytes()).expect(
            "Failed to print log message",
        );
    }
}
