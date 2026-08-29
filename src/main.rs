#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;
use game_transcribe::{audio, config::ConfigStore, engine::EngineHandle, ui};
use std::time::Duration;

fn main() -> Result<()> {
    let store = ConfigStore::discover()?;
    let arguments = std::env::args().collect::<Vec<_>>();
    if arguments.iter().any(|argument| argument == "--self-check") {
        let config = store.load()?;
        config.validate()?;
        let microphones = audio::input_device_names()?;
        println!(
            "Game Transcribe self-check passed ({} microphone(s) available)",
            microphones.len()
        );
        return Ok(());
    }
    let engine = EngineHandle::spawn(store.clone());
    if arguments.iter().any(|argument| argument == "--ui-smoke") {
        ui::run_startup_smoke_test(engine, store, Duration::from_secs(1))
    } else {
        ui::run(engine, store)
    }
}
