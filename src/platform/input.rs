use std::{ffi::c_void, mem::size_of, thread, time::Duration};

use anyhow::{Context, Result, bail};
use windows::Win32::{
    Foundation::HWND,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetKeyboardLayout, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
            KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RETURN,
            VK_RWIN, VK_SHIFT, VkKeyScanExW,
        },
        WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, IsWindow},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowToken(isize);

impl WindowToken {
    pub fn capture() -> Option<Self> {
        // SAFETY: GetForegroundWindow has no preconditions.
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.0.is_null()).then_some(Self(hwnd.0 as isize))
    }

    pub fn is_foreground(self) -> bool {
        Self::capture() == Some(self)
    }

    pub fn exists(self) -> bool {
        // SAFETY: stale HWND values are valid inputs to IsWindow.
        unsafe { IsWindow(Some(self.hwnd())).as_bool() }
    }

    fn hwnd(self) -> HWND {
        HWND(self.0 as *mut c_void)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySpec(u16);

impl KeySpec {
    pub fn parse(value: &str) -> Result<Self> {
        let trimmed = value.trim();
        if trimmed.eq_ignore_ascii_case("enter") || trimmed.eq_ignore_ascii_case("return") {
            return Ok(Self(VK_RETURN.0));
        }
        let mut chars = trimmed.chars();
        let character = chars.next().context("key cannot be empty")?;
        if chars.next().is_some() || character.len_utf16() != 1 {
            bail!("key must be Enter or one keyboard character");
        }
        Ok(Self(character as u16))
    }
}

pub fn type_message(
    target: WindowToken,
    message: &str,
    open_key: KeySpec,
    submit_key: KeySpec,
    delay: Duration,
) -> Result<()> {
    if !target.exists() || !target.is_foreground() {
        bail!("target window is no longer focused");
    }
    if modifiers_are_down() {
        bail!("a modifier key is held");
    }
    let layout = target_keyboard_layout(target);
    send_spec(open_key, layout)?;
    sleep_delay(delay.saturating_mul(3));

    for character in message.chars() {
        if !target.is_foreground() {
            bail!("focus changed while typing");
        }
        send_character(character, layout)?;
        sleep_delay(delay);
    }
    if !target.is_foreground() {
        bail!("focus changed before submit");
    }
    send_spec(submit_key, layout)
}

fn target_keyboard_layout(target: WindowToken) -> windows::Win32::UI::Input::KeyboardAndMouse::HKL {
    // SAFETY: the HWND was validated with IsWindow and a null process-id pointer is permitted.
    let thread = unsafe { GetWindowThreadProcessId(target.hwnd(), None) };
    // SAFETY: querying an existing GUI thread's keyboard layout has no extra preconditions.
    unsafe { GetKeyboardLayout(thread) }
}

fn send_spec(
    spec: KeySpec,
    layout: windows::Win32::UI::Input::KeyboardAndMouse::HKL,
) -> Result<()> {
    if spec.0 == VK_RETURN.0 {
        return send_virtual_key(VK_RETURN, false);
    }
    let character = char::from_u32(u32::from(spec.0)).context("invalid configured key")?;
    send_character(character, layout)
}

fn send_character(
    character: char,
    layout: windows::Win32::UI::Input::KeyboardAndMouse::HKL,
) -> Result<()> {
    let mut utf16 = [0_u16; 2];
    let encoded = character.encode_utf16(&mut utf16);
    if encoded.len() != 1 {
        bail!("the target keyboard layout cannot type a supplementary Unicode character");
    }
    // SAFETY: the keyboard layout handle belongs to the target thread.
    let mapping = unsafe { VkKeyScanExW(encoded[0], layout) };
    if mapping == -1 {
        bail!("the target keyboard layout cannot type '{character}'");
    }
    let virtual_key = VIRTUAL_KEY((mapping as u16) & 0xff);
    let modifiers = ((mapping as u16) >> 8) & 0xff;
    if modifiers & 0b110 != 0 {
        bail!("typing '{character}' would require Ctrl or Alt");
    }
    let shift = modifiers & 1 != 0;
    send_virtual_key(virtual_key, shift)
}

fn send_virtual_key(key: VIRTUAL_KEY, shift: bool) -> Result<()> {
    let mut inputs = Vec::with_capacity(if shift { 4 } else { 2 });
    if shift {
        inputs.push(key_input(VK_SHIFT, false));
    }
    inputs.push(key_input(key, false));
    inputs.push(key_input(key, true));
    if shift {
        inputs.push(key_input(VK_SHIFT, true));
    }
    // SAFETY: `inputs` points to initialized INPUT records and the size is exact.
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        if shift {
            // Best-effort cleanup if Windows accepted a partial sequence after
            // the synthetic Shift-down event.
            let _ = unsafe { SendInput(&[key_input(VK_SHIFT, true)], size_of::<INPUT>() as i32) };
        }
        bail!(
            "Windows accepted only {sent} of {} keyboard events",
            inputs.len()
        );
    }
    Ok(())
}

fn key_input(key: VIRTUAL_KEY, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                dwFlags: if key_up {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                ..Default::default()
            },
        },
    }
}

fn modifiers_are_down() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .into_iter()
        .any(|key| {
            // SAFETY: all values are documented virtual-key codes.
            (unsafe { GetAsyncKeyState(i32::from(key.0)) }) < 0
        })
}

fn sleep_delay(delay: Duration) {
    if !delay.is_zero() {
        thread::sleep(delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_enter_and_single_character() {
        assert_eq!(KeySpec::parse("Enter").unwrap(), KeySpec(VK_RETURN.0));
        assert_eq!(KeySpec::parse("/").unwrap(), KeySpec('/' as u16));
        assert!(KeySpec::parse("Space bar").is_err());
    }
}
