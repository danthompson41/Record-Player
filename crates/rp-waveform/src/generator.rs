use crate::mipmap::{MipmapLevel, WaveformMipmap, WaveformPoint};

/// Generates waveform mipmaps from audio data
pub struct WaveformGenerator {
    /// Number of mipmap levels to generate
    num_levels: usize,
}

impl WaveformGenerator {
    pub fn new() -> Self {
        Self { num_levels: 12 } // Supports up to 4096x reduction
    }

    /// Generate a complete waveform mipmap from audio samples
    pub fn generate(&self, samples: &[f32], sample_rate: u32, channels: u16) -> WaveformMipmap {
        let total_frames = samples.len() / channels as usize;
        let mut mipmap = WaveformMipmap::new(sample_rate, total_frames as u64, channels as u8);

        // Generate level 0 (1:1 - one point per sample)
        // For efficiency, we start at a reasonable base reduction
        let base_reduction = 64; // Start at 64 samples per point

        let mut current_data = self.generate_level(samples, channels, base_reduction);
        mipmap.levels.push(MipmapLevel {
            reduction: base_reduction,
            data: current_data.clone(),
        });

        // Generate subsequent levels by 2x downsampling
        let mut reduction = base_reduction * 2;
        for _ in 1..self.num_levels {
            if current_data.len() < 2 {
                break;
            }

            current_data = self.downsample_2x(&current_data);
            mipmap.levels.push(MipmapLevel {
                reduction,
                data: current_data.clone(),
            });

            reduction *= 2;
        }

        mipmap
    }

    /// Generate a single level from raw audio samples
    fn generate_level(&self, samples: &[f32], channels: u16, reduction: u32) -> Vec<WaveformPoint> {
        let total_frames = samples.len() / channels as usize;
        let num_points = (total_frames + reduction as usize - 1) / reduction as usize;
        let mut points = Vec::with_capacity(num_points);

        for i in 0..num_points {
            let start_frame = i * reduction as usize;
            let end_frame = ((i + 1) * reduction as usize).min(total_frames);

            let mut min_val = f32::MAX;
            let mut max_val = f32::MIN;
            let mut sum_sq = 0.0f32;
            let mut count = 0;

            for frame in start_frame..end_frame {
                // Mix channels to mono for waveform display
                let mut mono = 0.0f32;
                for ch in 0..channels as usize {
                    mono += samples[frame * channels as usize + ch];
                }
                mono /= channels as f32;

                min_val = min_val.min(mono);
                max_val = max_val.max(mono);
                sum_sq += mono * mono;
                count += 1;
            }

            let rms = if count > 0 {
                (sum_sq / count as f32).sqrt()
            } else {
                0.0
            };

            points.push(WaveformPoint::new(
                min_val.clamp(-1.0, 1.0),
                max_val.clamp(-1.0, 1.0),
                rms.clamp(0.0, 1.0),
            ));
        }

        points
    }

    /// Downsample by 2x - combine pairs of points
    fn downsample_2x(&self, data: &[WaveformPoint]) -> Vec<WaveformPoint> {
        let num_points = data.len() / 2;
        let mut result = Vec::with_capacity(num_points);

        for i in 0..num_points {
            let a = &data[i * 2];
            let b = &data[i * 2 + 1];

            result.push(WaveformPoint {
                min: a.min.min(b.min),
                max: a.max.max(b.max),
                // RMS of combined RMS values (approximation). Compute in u64:
                // two u16 values squared and summed overflow u32 (65535² · 2).
                rms: (((a.rms as u64).pow(2) + (b.rms as u64).pow(2)) / 2).isqrt() as u16,
            });
        }

        result
    }
}

impl Default for WaveformGenerator {
    fn default() -> Self {
        Self::new()
    }
}
