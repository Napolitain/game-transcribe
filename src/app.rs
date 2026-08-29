use std::{
    io::{self, Write},
    time::Duration,
};

use anyhow::Result;

use crate::{audio, config::ConfigStore, engine::EngineHandle, ui};

pub fn run() -> Result<()> {
    let arguments = std::env::args().collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|argument| argument == "--version" || argument == "-V")
    {
        write_stdout(&format!("game-transcribe {}", env!("CARGO_PKG_VERSION")));
        return Ok(());
    }

    let store = ConfigStore::discover()?;
    if arguments.iter().any(|argument| argument == "--self-check") {
        let config = store.load()?;
        config.validate()?;
        let microphones = audio::input_device_names()?;
        write_stdout(&format!(
            "Game Transcribe self-check passed ({} microphone(s) available)",
            microphones.len()
        ));
        return Ok(());
    }
    let engine = EngineHandle::spawn(store.clone());
    if arguments.iter().any(|argument| argument == "--ui-smoke") {
        ui::run_startup_smoke_test(engine, store, Duration::from_secs(1))
    } else {
        ui::run(engine, store)
    }
}

fn write_stdout(message: &str) {
    let mut output = io::stdout().lock();
    let _ = writeln!(output, "{message}");
    let _ = output.flush();
}
