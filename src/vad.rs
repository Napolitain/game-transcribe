use std::collections::VecDeque;

pub const SAMPLE_RATE: usize = 16_000;
pub const FRAME_SAMPLES: usize = 320;
const FRAME_MS: u32 = 20;
const PRE_ROLL_FRAMES: usize = 15;
const START_FRAMES: u32 = 2;
const MIN_SPEECH_FRAMES: u32 = 5;

#[derive(Debug, PartialEq)]
pub enum VadEvent {
    SpeechStarted,
    Utterance(Vec<f32>),
    TooLong,
}

#[derive(Debug)]
pub struct VoiceDetector {
    absolute_threshold: f32,
    noise_floor: f32,
    silence_frames: u32,
    max_frames: u32,
    pre_roll: VecDeque<Vec<f32>>,
    utterance: Vec<f32>,
    speaking: bool,
    consecutive_voice: u32,
    trailing_silence: u32,
    speech_frames: u32,
    total_frames: u32,
}

impl VoiceDetector {
    pub fn new(silence_ms: u32, max_seconds: u32, threshold: f32) -> Self {
        Self {
            absolute_threshold: threshold,
            noise_floor: threshold / 3.0,
            silence_frames: (silence_ms / FRAME_MS).max(1),
            max_frames: max_seconds.saturating_mul(1_000) / FRAME_MS,
            pre_roll: VecDeque::with_capacity(PRE_ROLL_FRAMES),
            utterance: Vec::new(),
            speaking: false,
            consecutive_voice: 0,
            trailing_silence: 0,
            speech_frames: 0,
            total_frames: 0,
        }
    }

    pub fn process_frame(&mut self, frame: &[f32]) -> Option<VadEvent> {
        debug_assert_eq!(frame.len(), FRAME_SAMPLES);
        let rms =
            (frame.iter().map(|sample| sample * sample).sum::<f32>() / frame.len() as f32).sqrt();
        let voice_threshold = self.absolute_threshold.max(self.noise_floor * 3.0);
        let is_voice = rms >= voice_threshold;

        if !self.speaking {
            if !is_voice {
                self.noise_floor = self.noise_floor.mul_add(0.98, rms * 0.02).max(0.000_1);
            }
            self.push_pre_roll(frame);
            self.consecutive_voice = if is_voice {
                self.consecutive_voice + 1
            } else {
                0
            };
            if self.consecutive_voice < START_FRAMES {
                return None;
            }

            self.speaking = true;
            self.speech_frames = self.consecutive_voice;
            self.total_frames = self.pre_roll.len() as u32;
            self.utterance.clear();
            for buffered in &self.pre_roll {
                self.utterance.extend_from_slice(buffered);
            }
            self.pre_roll.clear();
            return Some(VadEvent::SpeechStarted);
        }

        self.utterance.extend_from_slice(frame);
        self.total_frames += 1;
        if is_voice {
            self.speech_frames += 1;
            self.trailing_silence = 0;
        } else {
            self.trailing_silence += 1;
        }

        if self.total_frames >= self.max_frames {
            self.reset();
            return Some(VadEvent::TooLong);
        }
        if self.trailing_silence < self.silence_frames {
            return None;
        }

        if self.speech_frames < MIN_SPEECH_FRAMES {
            self.reset();
            return None;
        }
        let keep = self
            .utterance
            .len()
            .saturating_sub(self.trailing_silence as usize * FRAME_SAMPLES)
            + FRAME_SAMPLES * 5;
        self.utterance.truncate(keep.min(self.utterance.len()));
        let completed = std::mem::take(&mut self.utterance);
        self.reset_state();
        Some(VadEvent::Utterance(completed))
    }

    pub fn reset(&mut self) {
        self.utterance.clear();
        self.pre_roll.clear();
        self.reset_state();
    }

    fn reset_state(&mut self) {
        self.speaking = false;
        self.consecutive_voice = 0;
        self.trailing_silence = 0;
        self.speech_frames = 0;
        self.total_frames = 0;
    }

    fn push_pre_roll(&mut self, frame: &[f32]) {
        if self.pre_roll.len() == PRE_ROLL_FRAMES {
            self.pre_roll.pop_front();
        }
        self.pre_roll.push_back(frame.to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(level: f32) -> Vec<f32> {
        vec![level; FRAME_SAMPLES]
    }

    #[test]
    fn detects_speech_then_silence() {
        let mut vad = VoiceDetector::new(200, 5, 0.01);
        for _ in 0..20 {
            assert!(vad.process_frame(&frame(0.001)).is_none());
        }
        assert!(vad.process_frame(&frame(0.08)).is_none());
        assert_eq!(
            vad.process_frame(&frame(0.08)),
            Some(VadEvent::SpeechStarted)
        );
        for _ in 0..8 {
            assert!(vad.process_frame(&frame(0.08)).is_none());
        }
        let mut completed = None;
        for _ in 0..10 {
            if let Some(event) = vad.process_frame(&frame(0.0)) {
                completed = Some(event);
            }
        }
        assert!(matches!(completed, Some(VadEvent::Utterance(samples)) if !samples.is_empty()));
    }

    #[test]
    fn rejects_overlong_utterance() {
        let mut vad = VoiceDetector::new(200, 2, 0.01);
        let mut result = None;
        for _ in 0..110 {
            if let Some(event) = vad.process_frame(&frame(0.08)) {
                let done = event == VadEvent::TooLong;
                result = Some(event);
                if done {
                    break;
                }
            }
        }
        assert_eq!(result, Some(VadEvent::TooLong));
    }
}
