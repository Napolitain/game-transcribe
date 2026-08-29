use std::{iter, path::Path};

use anyhow::{Context, Result};
use windows::{
    Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    core::PCWSTR,
};

pub fn replace_file(source: &Path, target: &Path) -> Result<()> {
    let source = wide(source.as_os_str().to_string_lossy().as_ref());
    let target = wide(target.as_os_str().to_string_lossy().as_ref());
    // SAFETY: both buffers are NUL-terminated and remain alive for the call.
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .context("failed to atomically replace file")?;
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(iter::once(0)).collect()
}
