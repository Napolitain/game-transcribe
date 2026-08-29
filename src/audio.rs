use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result, bail};
use cpal::{
    FromSample, Sample, SampleFormat, SizedSample, Stream,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use crossbeam_channel::Sender;

use crate::resample::LinearResampler;

#[derive(Debug)]
pub enum AudioEvent {
    Samples(Vec<f32>),
    Error(String),
}

pub struct AudioCapture {
    _stream: Stream,
    gate: Arc<AtomicBool>,
}

impl AudioCapture {
    pub fn start(device_name: Option<&str>, sender: Sender<AudioEvent>) -> Result<Self> {
        let host = cpal::default_host();
        let device = if let Some(wanted) = device_name.filter(|name| !name.trim().is_empty()) {
            host.input_devices()
                .context("failed to enumerate microphones")?
                .find(|device| {
                    device
                        .description()
                        .is_ok_and(|description| description.name() == wanted)
                })
                .with_context(|| format!("microphone '{wanted}' is not available"))?
        } else {
            host.default_input_device()
                .context("no default microphone is available")?
        };
        let supported = device
            .default_input_config()
            .context("failed to query the microphone's default format")?;
        let sample_format = supported.sample_format();
        let config = supported.config();
        let gate = Arc::new(AtomicBool::new(true));
        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(&device, config, &sender, &gate)?,
            SampleFormat::F64 => build_stream::<f64>(&device, config, &sender, &gate)?,
            SampleFormat::I8 => build_stream::<i8>(&device, config, &sender, &gate)?,
            SampleFormat::I16 => build_stream::<i16>(&device, config, &sender, &gate)?,
            SampleFormat::I24 => build_stream::<cpal::I24>(&device, config, &sender, &gate)?,
            SampleFormat::I32 => build_stream::<i32>(&device, config, &sender, &gate)?,
            SampleFormat::I64 => build_stream::<i64>(&device, config, &sender, &gate)?,
            SampleFormat::U8 => build_stream::<u8>(&device, config, &sender, &gate)?,
            SampleFormat::U16 => build_stream::<u16>(&device, config, &sender, &gate)?,
            SampleFormat::U24 => build_stream::<cpal::U24>(&device, config, &sender, &gate)?,
            SampleFormat::U32 => build_stream::<u32>(&device, config, &sender, &gate)?,
            SampleFormat::U64 => build_stream::<u64>(&device, config, &sender, &gate)?,
            unsupported => bail!("unsupported microphone sample format: {unsupported}"),
        };
        stream
            .play()
            .context("failed to start microphone capture")?;
        Ok(Self {
            _stream: stream,
            gate,
        })
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.gate.store(enabled, Ordering::Release);
    }
}

pub fn input_device_names() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let mut names = host
        .input_devices()
        .context("failed to enumerate microphones")?
        .filter_map(|device| {
            device
                .description()
                .ok()
                .map(|description| description.name().to_owned())
        })
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    Ok(names)
}

fn build_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    sender: &Sender<AudioEvent>,
    gate: &Arc<AtomicBool>,
) -> Result<Stream>
where
    T: SizedSample + Sample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    let sample_rate = config.sample_rate;
    let sample_sender = sender.clone();
    let error_sender = sender.clone();
    let enabled = Arc::clone(gate);
    let mut resampler = LinearResampler::new(sample_rate, 16_000);
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data, _| {
                if !enabled.load(Ordering::Acquire) {
                    return;
                }
                let converted: Vec<f32> = data
                    .iter()
                    .map(|sample| sample.to_sample::<f32>())
                    .collect();
                let samples = resampler.push_interleaved(&converted, channels);
                if !samples.is_empty() {
                    let _ = sample_sender.try_send(AudioEvent::Samples(samples));
                }
            },
            move |error| {
                let _ = error_sender.try_send(AudioEvent::Error(error.to_string()));
            },
            None,
        )
        .context("failed to open the microphone")
}
