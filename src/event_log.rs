use std::io::{self, Write};

use windows::Win32::System::SystemInformation::GetSystemTime;

const MAX_DETAIL_CHARS: usize = 2_048;

#[derive(Debug, Default)]
pub struct EventLog;

impl EventLog {
    pub const fn new() -> Self {
        Self
    }

    pub fn record(&self, event: &str, detail: &str) {
        let mut output = io::stdout().lock();
        let _ = writeln!(output, "{}", format_line(event, detail));
        let _ = output.flush();
    }
}

fn format_line(event: &str, detail: &str) -> String {
    format!(
        "{} | {} | {}",
        utc_timestamp(),
        one_line(event),
        one_line(detail)
    )
}

fn utc_timestamp() -> String {
    // SAFETY: GetSystemTime has no preconditions and returns a value in UTC.
    let time = unsafe { GetSystemTime() };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds
    )
}

fn one_line(value: &str) -> String {
    let mut result = String::with_capacity(value.len().min(MAX_DETAIL_CHARS));
    let mut previous_was_space = false;
    for character in value.chars().take(MAX_DETAIL_CHARS) {
        if character.is_control() || character.is_whitespace() {
            if !previous_was_space {
                result.push(' ');
                previous_was_space = true;
            }
        } else {
            result.push(character);
            previous_was_space = false;
        }
    }
    if value.chars().count() > MAX_DETAIL_CHARS {
        result.push_str(" [truncated]");
    }
    result.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_single_line_events() {
        let line = format_line("sentence", "defend\nthe\tbridge");
        assert!(line.ends_with("| sentence | defend the bridge"));
        assert!(!line.contains('\n'));
        assert!(!line.contains('\t'));
    }

    #[test]
    fn truncates_excessive_detail() {
        let detail = "x".repeat(MAX_DETAIL_CHARS + 1);
        assert!(one_line(&detail).ends_with(" [truncated]"));
    }
}
