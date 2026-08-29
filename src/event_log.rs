use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use windows::Win32::System::SystemInformation::GetSystemTime;

const LOG_NAME: &str = "events.log";
const BACKUP_NAME: &str = "events.log.1";
const MAX_LOG_BYTES: u64 = 256 * 1024;
const MAX_DETAIL_CHARS: usize = 2_048;

#[derive(Debug)]
pub struct EventLog {
    path: PathBuf,
    backup: PathBuf,
    max_bytes: u64,
}

impl EventLog {
    pub fn new(root: &Path) -> Self {
        Self::with_limit(root, MAX_LOG_BYTES)
    }

    fn with_limit(root: &Path, max_bytes: u64) -> Self {
        Self {
            path: root.join(LOG_NAME),
            backup: root.join(BACKUP_NAME),
            max_bytes,
        }
    }

    pub fn record(&self, event: &str, detail: &str) {
        let _ = self.try_record(event, detail);
    }

    fn try_record(&self, event: &str, detail: &str) -> std::io::Result<()> {
        let timestamp = utc_timestamp();
        let event = one_line(event);
        let detail = one_line(detail);
        let line = format!("{timestamp} | {event} | {detail}\n");

        if let Some(root) = self.path.parent() {
            fs::create_dir_all(root)?;
        }
        let current_bytes = fs::metadata(&self.path).map_or(0, |metadata| metadata.len());
        if current_bytes > 0 && current_bytes.saturating_add(line.len() as u64) > self.max_bytes {
            match fs::remove_file(&self.backup) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            fs::rename(&self.path, &self.backup)?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())
    }
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

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("game-transcribe-log-{name}-{}", std::process::id()))
    }

    #[test]
    fn writes_single_line_events() {
        let root = test_root("single-line");
        let _ = fs::remove_dir_all(&root);
        let log = EventLog::new(&root);
        log.record("sentence", "defend\nthe\tbridge");

        let contents = fs::read_to_string(root.join(LOG_NAME)).unwrap();
        assert!(contents.ends_with("| sentence | defend the bridge\n"));
        assert_eq!(contents.lines().count(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn keeps_one_bounded_backup() {
        let root = test_root("rotation");
        let _ = fs::remove_dir_all(&root);
        let log = EventLog::with_limit(&root, 100);
        for index in 0..8 {
            log.record("vad", &format!("event={index} padding=abcdefghij"));
        }

        assert!(root.join(LOG_NAME).exists());
        assert!(root.join(BACKUP_NAME).exists());
        assert!(fs::metadata(root.join(LOG_NAME)).unwrap().len() <= 100);
        assert!(fs::metadata(root.join(BACKUP_NAME)).unwrap().len() <= 100);
        let _ = fs::remove_dir_all(root);
    }
}
