/// Small streaming mono resampler. It uses linear interpolation and keeps the
/// fractional input position across device callback boundaries.
#[derive(Debug)]
pub struct LinearResampler {
    ratio: f64,
    position: f64,
    samples: Vec<f32>,
}

impl LinearResampler {
    pub fn new(input_rate: u32, output_rate: u32) -> Self {
        Self {
            ratio: f64::from(input_rate) / f64::from(output_rate),
            position: 0.0,
            samples: Vec::with_capacity(4096),
        }
    }

    pub fn push_interleaved(&mut self, input: &[f32], channels: usize) -> Vec<f32> {
        if channels == 0 {
            return Vec::new();
        }
        self.samples.reserve(input.len() / channels);
        for frame in input.chunks_exact(channels) {
            self.samples
                .push(frame.iter().sum::<f32>() / channels as f32);
        }

        let mut output = Vec::with_capacity(
            ((self.samples.len() as f64 / self.ratio).ceil() as usize).min(self.samples.len() * 4),
        );
        while self.position + 1.0 < self.samples.len() as f64 {
            let left = self.position.floor() as usize;
            let fraction = (self.position - left as f64) as f32;
            let value =
                self.samples[left] + (self.samples[left + 1] - self.samples[left]) * fraction;
            output.push(value);
            self.position += self.ratio;
        }

        let consumed = self.position.floor() as usize;
        if consumed > 0 {
            self.samples.drain(..consumed);
            self.position -= consumed as f64;
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsampling_preserves_duration_across_chunks() {
        let mut resampler = LinearResampler::new(48_000, 16_000);
        let first = resampler.push_interleaved(&vec![0.5; 24_000], 1);
        let second = resampler.push_interleaved(&vec![0.5; 24_000], 1);
        let total = first.len() + second.len();
        assert!((15_999..=16_001).contains(&total), "got {total}");
        assert!(
            first
                .into_iter()
                .chain(second)
                .all(|sample| (sample - 0.5).abs() < 1e-6)
        );
    }

    #[test]
    fn mixes_stereo_to_mono() {
        let mut resampler = LinearResampler::new(16_000, 16_000);
        let output = resampler.push_interleaved(&[1.0, -1.0, 0.5, 0.5, 0.0, 0.0], 2);
        assert_eq!(output, vec![0.0, 0.5]);
    }
}
