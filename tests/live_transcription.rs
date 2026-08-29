use std::{io::Read, path::PathBuf};

use game_transcribe::{
    model::{self, ModelKind},
    transcription::Transcriber,
};

#[test]
#[ignore = "downloads the 31 MB model and an official whisper.cpp sample"]
fn official_sample_transcribes_end_to_end() {
    let models = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("live-test-models");
    let model = model::ensure_installed(ModelKind::TinyQ5_1, &models).unwrap();

    let mut response =
        ureq::get("https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/samples/jfk.wav")
            .call()
            .unwrap();
    let mut wav = Vec::new();
    response
        .body_mut()
        .as_reader()
        .read_to_end(&mut wav)
        .unwrap();
    let audio = pcm16_mono_16khz(&wav);

    let transcript = Transcriber::load(&model)
        .unwrap()
        .transcribe(&audio, "en")
        .unwrap();
    let normalized = transcript.text.to_lowercase();
    assert!(
        normalized.contains("country") && normalized.contains("you"),
        "unexpected official-sample transcript: {}",
        transcript.text
    );
}

fn pcm16_mono_16khz(wav: &[u8]) -> Vec<f32> {
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    let mut offset = 12;
    let mut valid_format = false;
    let mut data = None;
    while offset + 8 <= wav.len() {
        let id = &wav[offset..offset + 4];
        let size = u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let start = offset + 8;
        let end = start + size;
        assert!(end <= wav.len());
        if id == b"fmt " {
            let format = u16::from_le_bytes(wav[start..start + 2].try_into().unwrap());
            let channels = u16::from_le_bytes(wav[start + 2..start + 4].try_into().unwrap());
            let rate = u32::from_le_bytes(wav[start + 4..start + 8].try_into().unwrap());
            let bits = u16::from_le_bytes(wav[start + 14..start + 16].try_into().unwrap());
            valid_format = format == 1 && channels == 1 && rate == 16_000 && bits == 16;
        } else if id == b"data" {
            data = Some(&wav[start..end]);
        }
        offset = end + (size & 1);
    }
    assert!(valid_format, "sample must be 16 kHz mono PCM16");
    data.unwrap()
        .chunks_exact(2)
        .map(|bytes| f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32_768.0)
        .collect()
}
