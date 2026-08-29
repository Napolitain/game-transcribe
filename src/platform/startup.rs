use std::{
    iter,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR},
        System::Registry::{
            HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
            RegCreateKeyExW, RegDeleteValueW, RegSetValueExW,
        },
    },
    core::{HRESULT, PCWSTR},
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "GameTranscribe";

pub fn apply(enabled: bool) -> Result<()> {
    let key_path = wide(RUN_KEY);
    let value_name = wide(VALUE_NAME);
    let mut key = Default::default();
    // SAFETY: all pointers are valid for the duration of these registry calls.
    let create_result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_path.as_ptr()),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    win32_result(create_result).context("failed to open the current-user startup registry key")?;

    let operation = if enabled {
        let current = std::env::current_exe().context("failed to locate the executable")?;
        let executable = startup_executable(&current);
        let command = wide(&format!("\"{}\"", executable.display()));
        let bytes =
            unsafe { std::slice::from_raw_parts(command.as_ptr().cast::<u8>(), command.len() * 2) };
        // SAFETY: the key is open and the command contains a terminating NUL.
        win32_result(unsafe {
            RegSetValueExW(key, PCWSTR(value_name.as_ptr()), None, REG_SZ, Some(bytes))
        })
        .context("failed to enable launch at login")
    } else {
        // A missing value is already the desired state.
        let result = unsafe { RegDeleteValueW(key, PCWSTR(value_name.as_ptr())) };
        if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            win32_result(result).context("failed to disable launch at login")
        }
    };
    // SAFETY: `key` was returned by RegCreateKeyExW and is closed exactly once.
    let _ = unsafe { RegCloseKey(key) };
    operation
}

fn startup_executable(current: &Path) -> PathBuf {
    let gui = current.with_file_name("game-transcribe-gui.exe");
    if gui.is_file() {
        gui
    } else {
        current.to_owned()
    }
}

fn win32_result(error: WIN32_ERROR) -> windows::core::Result<()> {
    if error == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(windows::core::Error::from_hresult(HRESULT::from_win32(
            error.0,
        )))
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn launch_at_login_prefers_the_gui_binary() {
        let root =
            std::env::temp_dir().join(format!("game-transcribe-startup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let console = root.join("game-transcribe.exe");
        let gui = root.join("game-transcribe-gui.exe");

        assert_eq!(startup_executable(&console), console.clone());
        fs::write(&gui, []).unwrap();
        assert_eq!(startup_executable(&console), gui);
        let _ = fs::remove_dir_all(root);
    }
}
