use std::{
    sync::{Arc, RwLock},
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, bounded, select};

use crate::{
    audio::{AudioCapture, AudioEvent},
    config::{AppConfig, ConfigStore},
    event_log::EventLog,
    message::detect_phrases,
    model,
    platform::input::{self, KeySpec, WindowToken},
    transcription::Transcriber,
    vad::{FRAME_SAMPLES, SAMPLE_RATE, VadEvent, VoiceDetector},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    Starting,
    InstallingModel,
    Listening,
    HearingSpeech,
    Transcribing,
    WaitingForFocus,
    Typing,
    Sent,
    Paused,
    Rejected,
    Error,
}

impl AppState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Starting => "Starting",
            Self::InstallingModel => "Installing model",
            Self::Listening => "Listening",
            Self::HearingSpeech => "Hearing speech",
            Self::Transcribing => "Transcribing",
            Self::WaitingForFocus => "Waiting for focus",
            Self::Typing => "Typing",
            Self::Sent => "Message sent",
            Self::Paused => "Paused",
            Self::Rejected => "Message rejected",
            Self::Error => "Error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Status {
    pub state: AppState,
    pub detail: Option<String>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            state: AppState::Starting,
            detail: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Control {
    TogglePause,
    CancelPending,
    Reload,
    Shutdown,
}

#[derive(Clone)]
pub struct EngineHandle {
    control: Sender<Control>,
    status: Arc<RwLock<Status>>,
}

impl EngineHandle {
    pub fn spawn(store: ConfigStore) -> Self {
        let (control, receiver) = bounded(16);
        let status = Arc::new(RwLock::new(Status::default()));
        let worker_status = Arc::clone(&status);
        thread::Builder::new()
            .name("voice-engine".to_owned())
            .spawn(move || run_worker(store, receiver, worker_status))
            .expect("failed to start voice engine thread");
        Self { control, status }
    }

    pub fn send(&self, command: Control) {
        let _ = self.control.try_send(command);
    }

    pub fn status(&self) -> Status {
        self.status.read().map_or_else(
            |_| Status {
                state: AppState::Error,
                detail: Some("status lock was poisoned".to_owned()),
            },
            |status| status.clone(),
        )
    }
}

fn run_worker(store: ConfigStore, controls: Receiver<Control>, status: Arc<RwLock<Status>>) {
    let events = EventLog::new(store.root());
    loop {
        set_status(&status, AppState::Starting, None);
        let config = match store.load() {
            Ok(config) => config,
            Err(error) => {
                set_error(&status, &error);
                if wait_for_reload(&controls) {
                    continue;
                }
                return;
            }
        };
        set_status(
            &status,
            AppState::InstallingModel,
            Some("Verifying or downloading the selected local model".to_owned()),
        );
        let model_path = match model::ensure_installed(config.model, &store.models_dir()) {
            Ok(path) => path,
            Err(error) => {
                set_error(&status, &error);
                if wait_for_reload(&controls) {
                    continue;
                }
                return;
            }
        };
        let transcriber = match Transcriber::load(&model_path) {
            Ok(transcriber) => transcriber,
            Err(error) => {
                set_error(&status, &error);
                if wait_for_reload(&controls) {
                    continue;
                }
                return;
            }
        };
        match run_session(&config, &controls, &status, &transcriber, &events) {
            SessionExit::Reload => continue,
            SessionExit::Shutdown => return,
            SessionExit::Failed(error) => {
                set_error(&status, &error);
                if wait_for_reload(&controls) {
                    continue;
                }
                return;
            }
        }
    }
}

enum SessionExit {
    Reload,
    Shutdown,
    Failed(anyhow::Error),
}

fn run_session(
    config: &AppConfig,
    controls: &Receiver<Control>,
    status: &Arc<RwLock<Status>>,
    transcriber: &Transcriber,
    events: &EventLog,
) -> SessionExit {
    let (audio_sender, audio_receiver) = bounded(32);
    let capture = match AudioCapture::start(config.microphone.as_deref(), audio_sender) {
        Ok(capture) => capture,
        Err(error) => return SessionExit::Failed(error),
    };
    let mut vad = VoiceDetector::new(
        config.silence_ms,
        config.max_message_seconds,
        config.vad_threshold,
    );
    let mut frame_buffer = Vec::<f32>::with_capacity(FRAME_SAMPLES * 2);
    let mut target = None;
    let mut vad_active = false;
    let mut paused = false;
    set_status(status, AppState::Listening, None);

    loop {
        select! {
            recv(controls) -> command => match command {
                Ok(Control::Shutdown) | Err(_) => return SessionExit::Shutdown,
                Ok(Control::Reload) => return SessionExit::Reload,
                Ok(Control::TogglePause) => {
                    paused = !paused;
                    capture.set_enabled(!paused);
                    vad.reset();
                    frame_buffer.clear();
                    target = None;
                    if vad_active {
                        events.record("vad", "end reason=paused");
                        vad_active = false;
                    }
                    set_status(status, if paused { AppState::Paused } else { AppState::Listening }, None);
                }
                Ok(Control::CancelPending) => {
                    vad.reset();
                    frame_buffer.clear();
                    target = None;
                    if vad_active {
                        events.record("vad", "end reason=cancelled");
                        vad_active = false;
                    }
                    set_status(status, if paused { AppState::Paused } else { AppState::Listening }, None);
                }
            },
            recv(audio_receiver) -> event => {
                if paused {
                    continue;
                }
                match event {
                    Ok(AudioEvent::Samples(samples)) => {
                        frame_buffer.extend_from_slice(&samples);
                        while frame_buffer.len() >= FRAME_SAMPLES {
                            let frame: Vec<f32> = frame_buffer.drain(..FRAME_SAMPLES).collect();
                            match vad.process_frame(&frame) {
                                Some(VadEvent::SpeechStarted) => {
                                    target = WindowToken::capture();
                                    vad_active = true;
                                    events.record("vad", "start");
                                    set_status(status, AppState::HearingSpeech, None);
                                }
                                Some(VadEvent::TooLong) => {
                                    target = None;
                                    vad_active = false;
                                    events.record("vad", "end reason=too_long");
                                    set_status(status, AppState::Rejected, Some("Utterance exceeded the configured limit".to_owned()));
                                }
                                Some(VadEvent::Utterance(audio)) => {
                                    vad_active = false;
                                    let duration_ms = audio.len().saturating_mul(1_000) / SAMPLE_RATE;
                                    events.record("vad", &format!("end duration_ms={duration_ms}"));
                                    capture.set_enabled(false);
                                    let outcome = process_utterance(config, controls, status, transcriber, events, target, &audio);
                                    target = None;
                                    vad.reset();
                                    frame_buffer.clear();
                                    while audio_receiver.try_recv().is_ok() {}
                                    match outcome {
                                        PendingOutcome::Continue => {
                                            capture.set_enabled(true);
                                            set_status(status, AppState::Listening, None);
                                        }
                                        PendingOutcome::Pause => {
                                            paused = true;
                                            capture.set_enabled(false);
                                            set_status(status, AppState::Paused, None);
                                        }
                                        PendingOutcome::Reload => return SessionExit::Reload,
                                        PendingOutcome::Shutdown => return SessionExit::Shutdown,
                                    }
                                }
                                None => {}
                            }
                        }
                    }
                    Ok(AudioEvent::Error(message)) => {
                        return SessionExit::Failed(anyhow::anyhow!("microphone stream failed: {message}"));
                    }
                    Err(_) => return SessionExit::Failed(anyhow::anyhow!("microphone stream ended")),
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PendingOutcome {
    Continue,
    Pause,
    Reload,
    Shutdown,
}

fn process_utterance(
    config: &AppConfig,
    controls: &Receiver<Control>,
    status: &Arc<RwLock<Status>>,
    transcriber: &Transcriber,
    events: &EventLog,
    target: Option<WindowToken>,
    audio: &[f32],
) -> PendingOutcome {
    let Some(target) = target else {
        return PendingOutcome::Continue;
    };
    set_status(status, AppState::Transcribing, None);
    let transcript = match transcriber.transcribe(audio, &config.language) {
        Ok(transcript) => transcript,
        Err(error) => {
            set_error(status, &error);
            thread::sleep(Duration::from_millis(750));
            return PendingOutcome::Continue;
        }
    };

    if let Some(outcome) = drain_controls(controls) {
        return outcome;
    }
    let detection = detect_phrases(&transcript.text, &config.wake_phrase, &config.end_phrase);
    events.record(
        "markers",
        &format!(
            "start={} end={}",
            detection.start_detected, detection.end_detected
        ),
    );
    if transcript.text.is_empty()
        || transcript.confidence < config.min_confidence
        || transcript.no_speech_probability > 0.75
    {
        set_status(
            status,
            AppState::Rejected,
            Some("Recognition confidence was too low".to_owned()),
        );
        thread::sleep(Duration::from_millis(500));
        return PendingOutcome::Continue;
    }
    let Some(message) = detection.message else {
        return PendingOutcome::Continue;
    };
    events.record("sentence", message);

    if !target.exists() {
        events.record("send", "skipped reason=target_closed");
        return PendingOutcome::Continue;
    }
    if !target.is_foreground() {
        set_status(status, AppState::WaitingForFocus, None);
        match wait_for_focus(
            target,
            Duration::from_secs(u64::from(config.focus_timeout_seconds)),
            controls,
        ) {
            FocusWait::Ready => {}
            FocusWait::TargetClosed => {
                events.record("send", "skipped reason=target_closed");
                return PendingOutcome::Continue;
            }
            FocusWait::TimedOut => {
                events.record("send", "skipped reason=focus_timeout");
                return PendingOutcome::Continue;
            }
            FocusWait::Cancelled => {
                events.record("send", "skipped reason=cancelled");
                return PendingOutcome::Continue;
            }
            FocusWait::Pause => {
                events.record("send", "skipped reason=paused");
                return PendingOutcome::Pause;
            }
            FocusWait::Reload => {
                events.record("send", "skipped reason=settings_reloaded");
                return PendingOutcome::Reload;
            }
            FocusWait::Shutdown => {
                events.record("send", "skipped reason=shutdown");
                return PendingOutcome::Shutdown;
            }
        }
    }

    let open_key = match KeySpec::parse(&config.open_key) {
        Ok(key) => key,
        Err(error) => {
            events.record("send", "failed reason=invalid_open_key");
            set_error(status, &error);
            thread::sleep(Duration::from_millis(750));
            return PendingOutcome::Continue;
        }
    };
    let submit_key = match KeySpec::parse(&config.submit_key) {
        Ok(key) => key,
        Err(error) => {
            events.record("send", "failed reason=invalid_submit_key");
            set_error(status, &error);
            thread::sleep(Duration::from_millis(750));
            return PendingOutcome::Continue;
        }
    };
    set_status(status, AppState::Typing, None);
    if let Err(error) = input::type_message(
        target,
        message,
        open_key,
        submit_key,
        Duration::from_millis(u64::from(config.typing_delay_ms)),
    ) {
        events.record("send", &format!("failed error={error}"));
        set_error(status, &error);
        thread::sleep(Duration::from_millis(750));
        return PendingOutcome::Continue;
    }
    events.record("send", "success");
    set_status(status, AppState::Sent, None);
    thread::sleep(Duration::from_millis(350));
    PendingOutcome::Continue
}

fn drain_controls(controls: &Receiver<Control>) -> Option<PendingOutcome> {
    let mut cancelled = false;
    let mut pause_toggles = 0_u32;
    while let Ok(control) = controls.try_recv() {
        match control {
            Control::Shutdown => return Some(PendingOutcome::Shutdown),
            Control::Reload => return Some(PendingOutcome::Reload),
            Control::CancelPending => cancelled = true,
            Control::TogglePause => pause_toggles += 1,
        }
    }
    if pause_toggles % 2 == 1 {
        Some(PendingOutcome::Pause)
    } else {
        cancelled.then_some(PendingOutcome::Continue)
    }
}

enum FocusWait {
    Ready,
    TargetClosed,
    TimedOut,
    Cancelled,
    Pause,
    Reload,
    Shutdown,
}

fn wait_for_focus(
    target: WindowToken,
    timeout: Duration,
    controls: &Receiver<Control>,
) -> FocusWait {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !target.exists() {
            return FocusWait::TargetClosed;
        }
        if target.is_foreground() {
            return FocusWait::Ready;
        }
        match controls.recv_timeout(Duration::from_millis(50)) {
            Ok(Control::Shutdown) => return FocusWait::Shutdown,
            Ok(Control::Reload) => return FocusWait::Reload,
            Ok(Control::CancelPending) => return FocusWait::Cancelled,
            Ok(Control::TogglePause) => return FocusWait::Pause,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return FocusWait::Shutdown,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
    }
    FocusWait::TimedOut
}

fn wait_for_reload(controls: &Receiver<Control>) -> bool {
    loop {
        match controls.recv() {
            Ok(Control::Reload) => return true,
            Ok(Control::Shutdown) | Err(_) => return false,
            Ok(Control::TogglePause | Control::CancelPending) => {}
        }
    }
}

fn set_error(status: &Arc<RwLock<Status>>, error: &anyhow::Error) {
    set_status(status, AppState::Error, Some(format!("{error:#}")));
}

fn set_status(status: &Arc<RwLock<Status>>, state: AppState, detail: Option<String>) {
    if let Ok(mut status) = status.write() {
        status.state = state;
        status.detail = detail;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_during_transcription_remains_paused() {
        let (sender, receiver) = bounded(4);
        sender.send(Control::TogglePause).unwrap();
        assert_eq!(drain_controls(&receiver), Some(PendingOutcome::Pause));
    }

    #[test]
    fn two_pause_toggles_cancel_each_other() {
        let (sender, receiver) = bounded(4);
        sender.send(Control::TogglePause).unwrap();
        sender.send(Control::TogglePause).unwrap();
        assert_eq!(drain_controls(&receiver), None);
    }
}
