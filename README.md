# Game Transcribe

Game Transcribe is a lightweight Windows 11 tray application that turns a spoken message between configurable start and end phrases into normal keyboard input for game chat. Audio and recognition stay on the machine.

The default flow is:

1. Leave the app listening in the notification area.
2. Say `Viper`, the message, then `over` — for example, `Viper, defend the bridge, over`.
3. After 700 ms of silence, the app recognizes the utterance locally and requires both marker phrases at the transcript boundaries.
4. It strips `Viper` and `over`. If the original foreground window is still focused, it sends `Enter`, types only the message, and sends `Enter` again.

If focus changed during recognition, the message waits for the exact original window for up to 10 seconds. It is discarded rather than redirected to another application.

## Install with Scoop

On Windows, add the Napolitain bucket and install the current release:

```powershell
scoop bucket add napolitain https://github.com/Napolitain/scoop
scoop install napolitain/game-transcribe
```

Start `Game Transcribe` from the Start Menu or run `game-transcribe`. The first launch downloads and verifies the selected local recognition model.

## Safety and privacy

- No cloud speech service, local HTTP server, overlay, clipboard access, process injection, or game-memory access.
- No audio recordings or rejected transcripts are written to disk. The bounded event log records accepted message text as described below.
- Only one message can be pending. Microphone forwarding is disabled during recognition, focus waiting, and typing.
- Messages without the configured start phrase at the beginning and end phrase at the end are ignored.
- Held modifiers, unsupported keyboard-layout characters, focus changes, incomplete `SendInput` calls, and low-confidence recognition all prevent submission.
- The app never elevates itself or attempts to bypass anti-cheat. Windows UIPI and individual games can reject synthesized input.

## Build and run

Prerequisites:

- Windows 11 x64
- Rust 1.88 or newer with the MSVC target
- CMake, Ninja, and LLVM (`clang-cl` plus `libclang`)

The repository pins CMake's native `whisper.cpp` build to Ninja/clang-cl so it also works when a Visual Studio release is newer than the generators known by the installed CMake.

```powershell
cargo test --all-targets
cargo run
```

On first launch, the app downloads the selected quantized model from the official `whisper.cpp` model repository and verifies its pinned SHA-256 digest. The default multilingual Tiny Q5 model is about 31 MB. Once installed, normal operation is offline.

For a read-only microphone/configuration check that does not start the tray engine or download a model:

```powershell
cargo run -- --self-check
```

Create an optimized GUI binary with:

```powershell
cargo build --release
```

An opt-in live integration test downloads the Tiny Q5 model, verifies its checksum, loads it, and transcribes the official `whisper.cpp` sample end to end:

```powershell
cargo test --test live_transcription -- --ignored
```

The result is `target\release\game-transcribe.exe`. Release builds do not open a console window.

## Tray controls

Right-click the notification icon to pause or resume listening, cancel a pending message, open settings, or exit. Hovering the icon shows the current state or a generic diagnostic. Double-clicking opens settings.

Settings cover:

- microphone and recognition language code;
- start phrase, end phrase, and Tiny/Base quantized model;
- end-of-utterance silence and maximum speech duration;
- inter-key typing delay and focus-return timeout;
- open-chat and submit keys (`Enter` or one layout-mappable character);
- optional current-user launch at login.

Settings and verified models live under the app's Windows local application-data directory. Launch-at-login uses the current user's `Run` registry key and never requires administrator rights.

## Minimal event log

The app writes a compact UTF-8 event log to `%LOCALAPPDATA%\GameTranscribe\GameTranscribe\data\events.log`. It records only:

- VAD start and end, including completed audio duration or rejection reason;
- whether the configured start and end phrases were detected;
- the selected sentence after both phrases are stripped;
- whether sending succeeded, failed, or was skipped.

Each entry has a UTC timestamp and is kept on one line. The current log rotates at 256 KiB to `events.log.1`; only those two files are retained, limiting normal log storage to roughly 512 KiB. Accepted sentence text is therefore stored locally until rotation, while audio and rejected transcript text are never logged.

## Compatibility notes

Typing uses the active keyboard layout of the target window. This deliberately avoids clipboard paste and Unicode packet injection, but it means a character that the target layout cannot physically produce is rejected. Supplementary Unicode characters and characters requiring Ctrl/Alt are rejected to avoid triggering shortcuts.

Start compatibility testing in Notepad, then test each game in windowed, borderless, and fullscreen modes. Some games and anti-cheat systems independently reject `SendInput`; the app makes no attempt to work around that behavior.

## Verification

The test suite covers configuration migration and validation, strict start/end phrase matching, bounded event-log rotation, streaming resampling, VAD utterance boundaries and duration rejection, model checksum verification, and key parsing. The standard release gate is:

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
```
