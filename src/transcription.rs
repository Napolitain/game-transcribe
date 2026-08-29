use std::path::Path;

use anyhow::{Context, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Debug)]
pub struct Transcript {
    pub text: String,
    pub confidence: f32,
    pub no_speech_probability: f32,
}

pub struct Transcriber {
    context: WhisperContext,
}

impl Transcriber {
    pub fn load(model_path: &Path) -> Result<Self> {
        // No logging backend is enabled, so these hooks discard upstream
        // whisper.cpp/GGML output. Debug token traces can contain transcript
        // text and must never reach stderr or a persisted log collector.
        whisper_rs::install_logging_hooks();
        let model = model_path
            .to_str()
            .context("model path is not valid Unicode")?;
        let context = WhisperContext::new_with_params(model, WhisperContextParameters::default())
            .context("failed to initialize the local Whisper model")?;
        Ok(Self { context })
    }

    pub fn transcribe(&self, audio: &[f32], language: &str) -> Result<Transcript> {
        let mut state = self
            .context
            .create_state()
            .context("failed to create Whisper state")?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(recommended_threads());
        params.set_language(Some(language));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_single_segment(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        state
            .full(params, audio)
            .context("local speech recognition failed")?;

        let mut text = String::new();
        let mut probability_sum = 0.0_f32;
        let mut probability_count = 0_u32;
        let mut no_speech = 0.0_f32;
        let mut segment_count = 0_u32;
        for segment in state.as_iter() {
            text.push_str(segment.to_str_lossy()?.as_ref());
            no_speech += segment.no_speech_probability();
            segment_count += 1;
            for token_index in 0..segment.n_tokens() {
                if let Some(token) = segment.get_token(token_index) {
                    let probability = token.token_probability();
                    if probability.is_finite() {
                        probability_sum += probability;
                        probability_count += 1;
                    }
                }
            }
        }
        Ok(Transcript {
            text: text.trim().to_owned(),
            confidence: if probability_count == 0 {
                0.0
            } else {
                probability_sum / probability_count as f32
            },
            no_speech_probability: if segment_count == 0 {
                1.0
            } else {
                no_speech / segment_count as f32
            },
        })
    }
}

fn recommended_threads() -> i32 {
    std::thread::available_parallelism()
        .map_or(2, usize::from)
        .saturating_sub(1)
        .clamp(1, 8) as i32
}
