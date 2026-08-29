use std::{
    ffi::c_void,
    iter,
    mem::size_of,
    sync::OnceLock,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::BST_CHECKED,
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
                Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, CB_ADDSTRING, CB_GETCURSEL,
                CB_SETCURSEL, CBS_DROPDOWNLIST, CREATESTRUCTW, CW_USEDEFAULT, CreatePopupMenu,
                CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
                GWLP_USERDATA, GetCursorPos, GetDlgItemTextW, GetMessageW, GetWindowLongPtrW,
                HMENU, IDC_ARROW, IDI_APPLICATION, IsWindow, LoadCursorW, LoadIconW, MB_ICONERROR,
                MB_OK, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, MessageBoxW, PostQuitMessage,
                RegisterClassW, SW_SHOW, SendMessageW, SetForegroundWindow, SetTimer,
                SetWindowLongPtrW, SetWindowTextW, ShowWindow, TPM_RIGHTBUTTON, TrackPopupMenu,
                TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND,
                WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONDBLCLK, WM_NCCREATE, WM_NCDESTROY,
                WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_OVERLAPPED,
                WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
            },
        },
    },
    core::{PCWSTR, w},
};

use crate::{
    APP_NAME, audio,
    config::{AppConfig, ConfigStore},
    engine::{AppState, Control, EngineHandle},
    model::ModelKind,
    platform::startup,
};

const TRAY_CALLBACK: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
const TIMER_ID: usize = 1;
const MENU_TOGGLE: usize = 1001;
const MENU_CANCEL: usize = 1002;
const MENU_SETTINGS: usize = 1003;
const MENU_EXIT: usize = 1004;

const FIELD_MICROPHONE: i32 = 2000;
const FIELD_LANGUAGE: i32 = 2001;
const FIELD_WAKE: i32 = 2002;
const FIELD_MODEL: i32 = 2003;
const FIELD_SILENCE: i32 = 2004;
const FIELD_MAX_SECONDS: i32 = 2005;
const FIELD_DELAY: i32 = 2006;
const FIELD_FOCUS: i32 = 2007;
const FIELD_OPEN_KEY: i32 = 2008;
const FIELD_SUBMIT_KEY: i32 = 2009;
const FIELD_STARTUP: i32 = 2010;
const BUTTON_SAVE: usize = 1;
const BUTTON_CANCEL: usize = 2;

struct UiState {
    engine: EngineHandle,
    store: ConfigStore,
    tray_added: bool,
    last_tooltip: String,
    exit_at: Option<Instant>,
    settings_open: bool,
}

pub fn run(engine: EngineHandle, store: ConfigStore) -> Result<()> {
    run_inner(engine, store, None)
}

#[doc(hidden)]
pub fn run_startup_smoke_test(
    engine: EngineHandle,
    store: ConfigStore,
    duration: Duration,
) -> Result<()> {
    run_inner(engine, store, Some(Instant::now() + duration))
}

fn run_inner(engine: EngineHandle, store: ConfigStore, exit_at: Option<Instant>) -> Result<()> {
    // SAFETY: passing None requests the module for this process.
    let module = unsafe { GetModuleHandleW(None) }.context("failed to get application module")?;
    let instance = HINSTANCE(module.0);
    let class_name = wide("GameTranscribe.TrayWindow");
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.context("failed to load cursor")?;
    let class = WNDCLASSW {
        hInstance: instance,
        lpszClassName: PCWSTR(class_name.as_ptr()),
        lpfnWndProc: Some(tray_window_proc),
        hCursor: cursor,
        ..Default::default()
    };
    // SAFETY: `class` and its class-name buffer remain valid for registration.
    if unsafe { RegisterClassW(&class) } == 0 {
        bail!("failed to register the tray window class");
    }

    let state = Box::new(UiState {
        engine,
        store,
        tray_added: false,
        last_tooltip: String::new(),
        exit_at,
        settings_open: false,
    });
    let state_ptr = Box::into_raw(state);
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            w!("Game Transcribe"),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            None,
            None,
            Some(instance),
            Some(state_ptr.cast()),
        )
    };
    let window = match window {
        Ok(window) => window,
        Err(error) => {
            // SAFETY: ownership was not transferred to a window when creation failed.
            unsafe { drop(Box::from_raw(state_ptr)) };
            return Err(error).context("failed to create tray window");
        }
    };
    if let Err(error) = add_tray_icon(window) {
        let _ = unsafe { DestroyWindow(window) };
        return Err(error);
    }
    // SAFETY: the pointer remains owned by the live window until WM_NCDESTROY.
    unsafe { (*state_ptr).tray_added = true };
    unsafe { SetTimer(Some(window), TIMER_ID, 400, None) };

    let mut message = MSG::default();
    loop {
        // SAFETY: `message` is valid output storage; no message filter is requested.
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 == -1 {
            bail!("Windows message loop failed");
        }
        if result.0 == 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

unsafe extern "system" fn tray_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // SAFETY: WM_NCCREATE supplies a valid CREATESTRUCTW for the duration of this call.
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    let state_ptr = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut UiState;

    match message {
        TRAY_CALLBACK => {
            let event = lparam.0 as u32;
            if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                if !state_ptr.is_null() {
                    let state = unsafe { &mut *state_ptr };
                    let _ = show_tray_menu(window, state);
                }
                return LRESULT(0);
            }
            if event == WM_LBUTTONDBLCLK {
                if !state_ptr.is_null() {
                    let state = unsafe { &mut *state_ptr };
                    open_settings(window, state);
                }
                return LRESULT(0);
            }
        }
        WM_COMMAND => {
            if !state_ptr.is_null() {
                let state = unsafe { &mut *state_ptr };
                match wparam.0 & 0xffff {
                    MENU_TOGGLE => state.engine.send(Control::TogglePause),
                    MENU_CANCEL => state.engine.send(Control::CancelPending),
                    MENU_SETTINGS => {
                        open_settings(window, state);
                    }
                    MENU_EXIT => {
                        let _ = unsafe { DestroyWindow(window) };
                    }
                    _ => {}
                }
            }
            return LRESULT(0);
        }
        WM_TIMER => {
            if wparam.0 == TIMER_ID && !state_ptr.is_null() {
                let state = unsafe { &mut *state_ptr };
                if state
                    .exit_at
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    let _ = unsafe { DestroyWindow(window) };
                    return LRESULT(0);
                }
                update_tooltip(window, state);
            }
            return LRESULT(0);
        }
        WM_DESTROY => {
            if !state_ptr.is_null() {
                let state = unsafe { &mut *state_ptr };
                state.engine.send(Control::Shutdown);
                if state.tray_added {
                    delete_tray_icon(window);
                    state.tray_added = false;
                }
            }
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        WM_NCDESTROY if !state_ptr.is_null() => {
            unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, 0) };
            unsafe { drop(Box::from_raw(state_ptr)) };
        }
        _ => {}
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn add_tray_icon(window: HWND) -> Result<()> {
    let icon = unsafe { LoadIconW(None, IDI_APPLICATION) }.context("failed to load tray icon")?;
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: TRAY_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: TRAY_CALLBACK,
        hIcon: icon,
        ..Default::default()
    };
    copy_wide_array(&mut data.szTip, APP_NAME);
    if !unsafe { Shell_NotifyIconW(NIM_ADD, &data).as_bool() } {
        bail!("Windows refused to add the notification-area icon");
    }
    Ok(())
}

fn delete_tray_icon(window: HWND) {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: TRAY_ID,
        ..Default::default()
    };
    let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
}

fn update_tooltip(window: HWND, state: &mut UiState) {
    let status = state.engine.status();
    let tooltip = status.detail.as_ref().map_or_else(
        || format!("{APP_NAME} — {}", status.state.label()),
        |detail| format!("{APP_NAME} — {}: {detail}", status.state.label()),
    );
    if tooltip == state.last_tooltip {
        return;
    }
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: TRAY_ID,
        uFlags: NIF_TIP,
        ..Default::default()
    };
    copy_wide_array(&mut data.szTip, &tooltip);
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    state.last_tooltip = tooltip;
}

fn show_tray_menu(window: HWND, state: &UiState) -> Result<()> {
    let menu = unsafe { CreatePopupMenu() }.context("failed to create tray menu")?;
    let status = state.engine.status();
    let status_text = wide(&format!("Status: {}", status.state.label()));
    let pause_text = wide(if status.state == AppState::Paused {
        "Resume"
    } else {
        "Pause"
    });
    let cancel_text = wide("Cancel pending message");
    let settings_text = wide("Settings...");
    let exit_text = wide("Exit");
    let append_result = (|| -> windows::core::Result<()> {
        unsafe {
            AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(status_text.as_ptr()))?;
            AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
            AppendMenuW(menu, MF_STRING, MENU_TOGGLE, PCWSTR(pause_text.as_ptr()))?;
            AppendMenuW(menu, MF_STRING, MENU_CANCEL, PCWSTR(cancel_text.as_ptr()))?;
            AppendMenuW(
                menu,
                MF_STRING,
                MENU_SETTINGS,
                PCWSTR(settings_text.as_ptr()),
            )?;
            AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?;
            AppendMenuW(menu, MF_STRING, MENU_EXIT, PCWSTR(exit_text.as_ptr()))?;
        }
        Ok(())
    })();
    if let Err(error) = append_result {
        let _ = unsafe { DestroyMenu(menu) };
        return Err(error).context("failed to populate tray menu");
    }
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.context("failed to locate pointer")?;
    unsafe {
        let _ = SetForegroundWindow(window);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, None, window, None);
    }
    let _ = unsafe { DestroyMenu(menu) };
    Ok(())
}

struct SettingsState {
    store: ConfigStore,
    engine: EngineHandle,
    microphones: Vec<String>,
}

fn open_settings(parent: HWND, state: &mut UiState) {
    if state.settings_open {
        return;
    }
    state.settings_open = true;
    let result = show_settings(parent, state.store.clone(), state.engine.clone());
    state.settings_open = false;
    if let Err(error) = result {
        show_error(parent, &format!("Could not open settings:\n\n{error:#}"));
    }
}

fn show_settings(parent: HWND, store: ConfigStore, engine: EngineHandle) -> Result<()> {
    let config = store.load().unwrap_or_default();
    let microphones = audio::input_device_names().unwrap_or_default();
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None) }?.0);
    register_settings_class(instance);
    let state = Box::new(SettingsState {
        store,
        engine,
        microphones,
    });
    let state_ptr = Box::into_raw(state);
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("GameTranscribe.SettingsWindow"),
            w!("Game Transcribe Settings"),
            WS_CAPTION | WS_SYSMENU,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            570,
            500,
            Some(parent),
            None,
            Some(instance),
            Some(state_ptr.cast()),
        )
    };
    let window = match window {
        Ok(window) => window,
        Err(error) => {
            unsafe { drop(Box::from_raw(state_ptr)) };
            return Err(error).context("failed to open settings window");
        }
    };
    if let Err(error) = create_settings_controls(window, instance, &config, unsafe { &*state_ptr })
    {
        let _ = unsafe { DestroyWindow(window) };
        return Err(error);
    }
    let _ = unsafe { ShowWindow(window, SW_SHOW) };

    let mut message = MSG::default();
    while unsafe { IsWindow(Some(window)).as_bool() } {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 <= 0 {
            if result.0 == 0 {
                unsafe { PostQuitMessage(message.wParam.0 as i32) };
            }
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

fn register_settings_class(instance: HINSTANCE) {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    REGISTERED.get_or_init(|| {
        let class = WNDCLASSW {
            hInstance: instance,
            lpszClassName: w!("GameTranscribe.SettingsWindow"),
            lpfnWndProc: Some(settings_window_proc),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) };
    });
}

fn create_settings_controls(
    window: HWND,
    instance: HINSTANCE,
    config: &AppConfig,
    state: &SettingsState,
) -> Result<()> {
    let labels = [
        ("Microphone", FIELD_MICROPHONE),
        ("Language code", FIELD_LANGUAGE),
        ("Wake phrase", FIELD_WAKE),
        ("Recognition model", FIELD_MODEL),
        ("End silence (ms)", FIELD_SILENCE),
        ("Maximum speech (sec)", FIELD_MAX_SECONDS),
        ("Typing delay (ms)", FIELD_DELAY),
        ("Focus wait (sec)", FIELD_FOCUS),
        ("Open-chat key", FIELD_OPEN_KEY),
        ("Submit key", FIELD_SUBMIT_KEY),
    ];
    for (row, (label, id)) in labels.into_iter().enumerate() {
        let y = 18 + row as i32 * 36;
        create_control(
            window,
            instance,
            w!("STATIC"),
            label,
            WS_CHILD | WS_VISIBLE,
            18,
            y + 4,
            165,
            22,
            0,
        )?;
        let style = if id == FIELD_MICROPHONE || id == FIELD_MODEL {
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(CBS_DROPDOWNLIST as u32)
        } else {
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER
        };
        create_control(
            window,
            instance,
            if id == FIELD_MICROPHONE || id == FIELD_MODEL {
                w!("COMBOBOX")
            } else {
                w!("EDIT")
            },
            "",
            style,
            190,
            y,
            345,
            300,
            id,
        )?;
    }
    create_control(
        window,
        instance,
        w!("BUTTON"),
        "Launch at login",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
        190,
        380,
        180,
        25,
        FIELD_STARTUP,
    )?;
    create_control(
        window,
        instance,
        w!("BUTTON"),
        "Save",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        360,
        420,
        80,
        28,
        BUTTON_SAVE as i32,
    )?;
    create_control(
        window,
        instance,
        w!("BUTTON"),
        "Cancel",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        455,
        420,
        80,
        28,
        BUTTON_CANCEL as i32,
    )?;

    set_text(window, FIELD_LANGUAGE, &config.language)?;
    set_text(window, FIELD_WAKE, &config.wake_phrase)?;
    set_text(window, FIELD_SILENCE, &config.silence_ms.to_string())?;
    set_text(
        window,
        FIELD_MAX_SECONDS,
        &config.max_message_seconds.to_string(),
    )?;
    set_text(window, FIELD_DELAY, &config.typing_delay_ms.to_string())?;
    set_text(
        window,
        FIELD_FOCUS,
        &config.focus_timeout_seconds.to_string(),
    )?;
    set_text(window, FIELD_OPEN_KEY, &config.open_key)?;
    set_text(window, FIELD_SUBMIT_KEY, &config.submit_key)?;

    let microphone_combo = get_control(window, FIELD_MICROPHONE)?;
    combo_add(microphone_combo, "Default microphone");
    for name in &state.microphones {
        combo_add(microphone_combo, name);
    }
    let microphone_index = config
        .microphone
        .as_ref()
        .and_then(|wanted| state.microphones.iter().position(|name| name == wanted))
        .map_or(0, |index| index + 1);
    unsafe {
        SendMessageW(
            microphone_combo,
            CB_SETCURSEL,
            Some(WPARAM(microphone_index)),
            None,
        )
    };

    let model_combo = get_control(window, FIELD_MODEL)?;
    for model in ModelKind::ALL {
        combo_add(model_combo, model.label());
    }
    unsafe {
        SendMessageW(
            model_combo,
            CB_SETCURSEL,
            Some(WPARAM(config.model.index())),
            None,
        )
    };
    let startup_check = get_control(window, FIELD_STARTUP)?;
    if config.launch_at_login {
        unsafe {
            SendMessageW(
                startup_check,
                BM_SETCHECK,
                Some(WPARAM(BST_CHECKED.0 as usize)),
                None,
            )
        };
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn create_control(
    parent: HWND,
    instance: HINSTANCE,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: i32,
) -> Result<HWND> {
    let text = wide(text);
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR(text.as_ptr()),
            style,
            x,
            y,
            width,
            height,
            Some(parent),
            Some(HMENU(id as isize as *mut c_void)),
            Some(instance),
            None,
        )
    }
    .context("failed to create settings control")
}

unsafe extern "system" fn settings_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    let state_ptr = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut SettingsState;
    match message {
        WM_COMMAND => match wparam.0 & 0xffff {
            BUTTON_SAVE => {
                if !state_ptr.is_null() {
                    let state = unsafe { &mut *state_ptr };
                    if let Err(error) = save_settings(window, state) {
                        show_error(window, &format!("Could not save settings:\n\n{error:#}"));
                    } else {
                        let _ = unsafe { DestroyWindow(window) };
                    }
                }
                return LRESULT(0);
            }
            BUTTON_CANCEL => {
                let _ = unsafe { DestroyWindow(window) };
                return LRESULT(0);
            }
            _ => {}
        },
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(window) };
            return LRESULT(0);
        }
        WM_NCDESTROY if !state_ptr.is_null() => {
            unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, 0) };
            unsafe { drop(Box::from_raw(state_ptr)) };
        }
        _ => {}
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn save_settings(window: HWND, state: &mut SettingsState) -> Result<()> {
    let mut config = state.store.load().unwrap_or_default();
    let microphone_index = combo_selection(get_control(window, FIELD_MICROPHONE)?);
    config.microphone = microphone_index
        .checked_sub(1)
        .and_then(|index| state.microphones.get(index).cloned());
    config.language = get_text(window, FIELD_LANGUAGE)?;
    config.wake_phrase = get_text(window, FIELD_WAKE)?;
    config.model = ModelKind::from_index(combo_selection(get_control(window, FIELD_MODEL)?));
    config.silence_ms = get_number(window, FIELD_SILENCE, "end silence")?;
    config.max_message_seconds = get_number(window, FIELD_MAX_SECONDS, "maximum speech")?;
    config.typing_delay_ms = get_number(window, FIELD_DELAY, "typing delay")?;
    config.focus_timeout_seconds = get_number(window, FIELD_FOCUS, "focus wait")?;
    config.open_key = get_text(window, FIELD_OPEN_KEY)?;
    config.submit_key = get_text(window, FIELD_SUBMIT_KEY)?;
    let startup_check = get_control(window, FIELD_STARTUP)?;
    config.launch_at_login =
        unsafe { SendMessageW(startup_check, BM_GETCHECK, None, None) }.0 as u32 == BST_CHECKED.0;
    config.validate()?;
    startup::apply(config.launch_at_login)?;
    state.store.save(&config)?;
    state.engine.send(Control::Reload);
    Ok(())
}

fn get_number(window: HWND, id: i32, label: &str) -> Result<u32> {
    get_text(window, id)?
        .parse::<u32>()
        .with_context(|| format!("{label} must be a whole number"))
}

fn get_text(window: HWND, id: i32) -> Result<String> {
    let mut buffer = [0_u16; 512];
    let length = unsafe { GetDlgItemTextW(window, id, &mut buffer) } as usize;
    Ok(String::from_utf16_lossy(&buffer[..length])
        .trim()
        .to_owned())
}

fn set_text(window: HWND, id: i32, value: &str) -> Result<()> {
    let control = get_control(window, id)?;
    let value = wide(value);
    unsafe { SetWindowTextW(control, PCWSTR(value.as_ptr())) }
        .context("failed to initialize setting")
}

fn get_control(window: HWND, id: i32) -> Result<HWND> {
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetDlgItem(Some(window), id) }
        .context("settings control was not found")
}

fn combo_add(combo: HWND, value: &str) {
    let value = wide(value);
    unsafe {
        SendMessageW(
            combo,
            CB_ADDSTRING,
            None,
            Some(LPARAM(value.as_ptr() as isize)),
        )
    };
}

fn combo_selection(combo: HWND) -> usize {
    let result = unsafe { SendMessageW(combo, CB_GETCURSEL, None, None) }.0;
    if result < 0 { 0 } else { result as usize }
}

fn show_error(window: HWND, message: &str) {
    let message = wide(message);
    unsafe {
        MessageBoxW(
            Some(window),
            PCWSTR(message.as_ptr()),
            w!("Game Transcribe"),
            MB_OK | MB_ICONERROR,
        )
    };
}

fn copy_wide_array<const N: usize>(destination: &mut [u16; N], value: &str) {
    destination.fill(0);
    for (slot, unit) in destination
        .iter_mut()
        .take(N.saturating_sub(1))
        .zip(value.encode_utf16())
    {
        *slot = unit;
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(iter::once(0)).collect()
}
